import { BN } from "@coral-xyz/anchor";
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  Ed25519Program,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import { Command } from "commander";
import {
  buildAggregatedSig,
  creditedBitsFromBitmap,
  parseAggregatedSigJson,
  signersBitmapFromOptions,
} from "../lib/agg-sig";
import { hashAgentRequestAuthFields, hashRequestAuthFields } from "../lib/auth";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findAgentEscrowPda,
  findAgentRoundPda,
  findDelegatePda,
  findGatewayPda,
  findProtocolConfigPda,
  findRegistryPda,
  findRegistryStatePda,
  findSubscriptionPda,
  findSubscriptionRoundPda,
} from "../lib/pdas";
import {
  bn,
  hexToBytes,
  loadKeypair,
  parseInteger,
  parsePublicKey,
  requireValue,
  writeJson,
} from "../lib/utils";

function registryNode(registry: any, bit: number): PublicKey {
  if (bit >= Number(registry.nodeCount)) {
    throw new Error(`credited bit ${bit} is outside registry node_count`);
  }
  return new PublicKey(Uint8Array.from(registry.nodes[bit]));
}

export function registerRoundCommands(program: Command): void {
  const round = program
    .command("round")
    .description("Settle, finalize, and dispute round receipts");

  round
    .command("settle-subscription")
    .description("Settle a subscription/delegate paid round")
    .requiredOption("--source-id <hex>", "32-byte source identifier")
    .requiredOption("--timestamp <number>", "canonical timestamp")
    .requiredOption("--signatures-required <number>", "quorum threshold")
    .requiredOption("--registry-version <number>", "registry version")
    .requiredOption(
      "--gateway-authority <pubkey>",
      "gateway authority pubkey (for PDA)"
    )
    .option("--value <hex>", "oracle value bytes (hex)", "0x" + "00".repeat(32))
    .option("--subscription-owner <pubkey>", "subscription owner")
    .option(
      "--consumer-wallet <path>",
      "consumer (owner or delegate) keypair that signs RequestAuth"
    )
    .option("--agg-sig <path>", "JSON AggregatedSig file")
    .option("--signers-bitmap <hex>", "32-byte credited signers bitmap")
    .option("--credited-bits <list>", "comma-separated credited bit indices")
    .option("--signer-count <number>", "first-N bits credited")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const consumer = options.consumerWallet
        ? loadKeypair(options.consumerWallet)
        : ctx.payer;
      const subscriptionOwner = options.subscriptionOwner
        ? parsePublicKey(options.subscriptionOwner, "subscription owner")
        : consumer.publicKey;
      const [subscriptionPda] = findSubscriptionPda(
        ctx.programId,
        subscriptionOwner
      );
      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority"
      );
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);

      const sourceId = hexToBytes(options.sourceId, 32, "source id");
      const timestamp = parseInteger(options.timestamp, "timestamp");
      const signaturesRequired = parseInteger(
        options.signaturesRequired,
        "signatures required"
      );
      const registryVersion = parseInteger(
        options.registryVersion,
        "registry version"
      );
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const value = hexToBytes(options.value, undefined, "value");
      const aggSig = options.aggSig
        ? parseAggregatedSigJson(options.aggSig)
        : buildAggregatedSig({
            signersBitmap: signersBitmapFromOptions(options),
          });
      const creditedBits = creditedBitsFromBitmap(
        Uint8Array.from(aggSig.signersBitmap)
      );
      const [roundRecordPda] = findSubscriptionRoundPda(
        ctx.programId,
        consumer.publicKey,
        sourceId,
        signaturesRequired,
        timestamp
      );

      const consumerDelegate = consumer.publicKey.equals(subscriptionOwner)
        ? null
        : findDelegatePda(
            ctx.programId,
            subscriptionOwner,
            consumer.publicKey
          )[0];

      const authHash = hashRequestAuthFields(
        sourceId,
        signaturesRequired,
        timestamp
      );
      const signature = await (ctx.program.methods as any)
        .settleRoundReceipt({
          timestamp: new BN(timestamp),
          sourceId: Array.from(sourceId),
          signaturesRequired,
          registryVersion,
          aggSig,
          value: Buffer.from(value),
          consumerAuthority: consumer.publicKey,
        })
        .accountsPartial({
          payer: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          registry: registryPda,
          subscription: subscriptionPda,
          consumerDelegate,
          gateway: gatewayPda,
          roundRecord: roundRecordPda,
          instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(
          creditedBits.map((bit) => ({
            pubkey: registryNode(registry, bit),
            isWritable: false,
            isSigner: false,
          }))
        )
        .preInstructions([
          Ed25519Program.createInstructionWithPrivateKey({
            privateKey: consumer.secretKey,
            message: authHash,
          }),
        ])
        .signers(
          consumer.publicKey.equals(ctx.wallet.publicKey) ? [] : [consumer]
        )
        .rpc();

      writeJson({
        signature,
        roundRecord: roundRecordPda.toBase58(),
        sourceId: `0x${Buffer.from(sourceId).toString("hex")}`,
        creditedBits,
      });
    });

  round
    .command("settle-agent")
    .description("Settle a self-funded / x402 agent round")
    .requiredOption("--source-id <hex>", "32-byte source identifier")
    .requiredOption("--canonical-timestamp <number>", "canonical timestamp")
    .requiredOption("--signatures-required <number>", "quorum threshold")
    .requiredOption("--registry-version <number>", "registry version")
    .requiredOption("--amount <amount>", "exact agent round price")
    .option(
      "--payer-wallet <path>",
      "x402 payer keypair (escrow authority); defaults to provider wallet"
    )
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet"
    )
    .option("--value <hex>", "oracle value bytes (hex)", "0x" + "00".repeat(32))
    .option("--agg-sig <path>", "JSON AggregatedSig file")
    .option("--signers-bitmap <hex>", "32-byte credited signers bitmap")
    .option("--credited-bits <list>", "comma-separated credited bit indices")
    .option("--signer-count <number>", "first-N bits credited")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const payer = options.payerWallet
        ? loadKeypair(options.payerWallet)
        : ctx.payer;
      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey
      );
      const [escrowPda] = findAgentEscrowPda(
        ctx.programId,
        payer.publicKey,
        gatewayPda
      );
      const escrowAta = getAssociatedTokenAddressSync(
        protocolConfig.usdcMint,
        escrowPda,
        true
      );

      const sourceId = hexToBytes(options.sourceId, 32, "source id");
      const canonicalTimestamp = parseInteger(
        options.canonicalTimestamp,
        "canonical timestamp"
      );
      const signaturesRequired = parseInteger(
        options.signaturesRequired,
        "signatures required"
      );
      const registryVersion = parseInteger(
        options.registryVersion,
        "registry version"
      );
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const amount = bn(options.amount, "amount");
      const value = hexToBytes(options.value, undefined, "value");
      const aggSig = options.aggSig
        ? parseAggregatedSigJson(options.aggSig)
        : buildAggregatedSig({
            signersBitmap: signersBitmapFromOptions(options),
          });
      const creditedBits = creditedBitsFromBitmap(
        Uint8Array.from(aggSig.signersBitmap)
      );
      const [roundRecordPda] = findAgentRoundPda(
        ctx.programId,
        payer.publicKey,
        gatewayPda,
        sourceId,
        signaturesRequired,
        canonicalTimestamp
      );

      const authHash = hashAgentRequestAuthFields(
        escrowPda,
        gatewayPda,
        sourceId,
        signaturesRequired,
        canonicalTimestamp,
        BigInt(amount.toString())
      );

      const signers = [];
      if (!gatewayAuthority.publicKey.equals(ctx.wallet.publicKey)) {
        signers.push(gatewayAuthority);
      }
      if (
        !payer.publicKey.equals(ctx.wallet.publicKey) &&
        !payer.publicKey.equals(gatewayAuthority.publicKey)
      ) {
        // Payer signs via Ed25519 pre-ix only; no Solana signature required unless fee payer.
      }

      const signature = await (ctx.program.methods as any)
        .settleAgentRound({
          authority: payer.publicKey,
          amount,
          sourceId: Array.from(sourceId),
          canonicalTimestamp: new BN(canonicalTimestamp),
          signaturesRequired,
          registryVersion,
          aggSig,
          value: Buffer.from(value),
        })
        .accountsPartial({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          registry: registryPda,
          gateway: gatewayPda,
          agentEscrow: escrowPda,
          usdcMint: protocolConfig.usdcMint,
          escrowAta,
          treasury: protocolConfig.treasury,
          tokenProgram: TOKEN_PROGRAM_ID,
          roundRecord: roundRecordPda,
          instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(
          creditedBits.map((bit) => ({
            pubkey: registryNode(registry, bit),
            isWritable: false,
            isSigner: false,
          }))
        )
        .preInstructions([
          Ed25519Program.createInstructionWithPrivateKey({
            privateKey: payer.secretKey,
            message: authHash,
          }),
        ])
        .signers(signers)
        .rpc();

      writeJson({
        signature,
        roundRecord: roundRecordPda.toBase58(),
        agentEscrow: escrowPda.toBase58(),
        escrowAta: escrowAta.toBase58(),
        sourceId: `0x${Buffer.from(sourceId).toString("hex")}`,
        creditedBits,
      });
    });

  round
    .command("finalize")
    .description("Finalize a pending round after the dispute window")
    .requiredOption("--round-record <pubkey>", "round receipt record address")
    .requiredOption(
      "--gateway-authority <pubkey>",
      "gateway authority pubkey (for PDA)"
    )
    .option(
      "--credited-bits <list>",
      "comma-separated credited bits (default: read from round record)"
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const roundRecord = parsePublicKey(options.roundRecord, "round record");
      const record = await fetchAccountOrNull<any>(
        ctx.program,
        "roundReceiptRecord",
        roundRecord
      );
      if (!record) {
        throw new Error(`Round record not found at ${roundRecord.toBase58()}`);
      }

      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority"
      );
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);
      const creditedBits = options.creditedBits
        ? options.creditedBits
            .split(",")
            .map((part: string) => parseInteger(part.trim(), "credited bit"))
        : creditedBitsFromBitmap(Uint8Array.from(record.signersBitmap));
      const registryVersion = Number(record.registryVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);

      const signature = await (ctx.program.methods as any)
        .finalizeRound()
        .accountsPartial({
          caller: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          registry: registryPda,
          roundRecord,
          gateway: gatewayPda,
        })
        .remainingAccounts(
          creditedBits.map((bit: number) => ({
            pubkey: registryNode(registry, bit),
            isWritable: true,
            isSigner: false,
          }))
        )
        .rpc();

      writeJson({
        signature,
        roundRecord: roundRecord.toBase58(),
        creditedBits,
      });
    });

  round
    .command("dispute")
    .description("Dispute a pending round with a corrected AggregatedSig")
    .requiredOption("--round-record <pubkey>", "round receipt record address")
    .requiredOption(
      "--gateway-authority <pubkey>",
      "gateway authority pubkey (for PDA)"
    )
    .requiredOption("--agg-sig <path>", "JSON AggregatedSig used as challenge")
    .option("--challenger-usdc <pubkey>", "challenger USDC ATA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const roundRecord = parsePublicKey(options.roundRecord, "round record");
      const record = await fetchAccountOrNull<any>(
        ctx.program,
        "roundReceiptRecord",
        roundRecord
      );
      if (!record)
        throw new Error(`Round record not found at ${roundRecord.toBase58()}`);
      const registryVersion = Number(record.registryVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority"
      );
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);
      const aggSig = parseAggregatedSigJson(
        requireValue(options.aggSig, "Missing --agg-sig")
      );
      const challengerUsdc = options.challengerUsdc
        ? parsePublicKey(options.challengerUsdc, "challenger USDC")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );

      const signature = await (ctx.program.methods as any)
        .disputeRound(aggSig)
        .accountsPartial({
          challenger: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          registry: registryPda,
          roundRecord,
          gateway: gatewayPda,
          treasury: protocolConfig.treasury,
          challengerUsdc,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .remainingAccounts(
          creditedBitsFromBitmap(Uint8Array.from(aggSig.signersBitmap)).map(
            (bit) => ({
              pubkey: registryNode(registry, bit),
              isWritable: false,
              isSigner: false,
            })
          )
        )
        .rpc();

      writeJson({
        signature,
        roundRecord: roundRecord.toBase58(),
      });
    });
}
