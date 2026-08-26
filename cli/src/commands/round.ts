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
import { hashRequestAuthFields } from "../lib/auth";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  derivePayerId,
  findDelegatePda,
  findGatewayBondAta,
  findGatewayPda,
  findProtocolConfigPda,
  findRegistryPda,
  findRegistryStatePda,
  findSubscriptionPda,
  findSubscriptionRoundPda,
  findX402RoundPda,
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
        subscriptionOwner,
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
        .settleSubscriptionRound({
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
    .command("settle-x402")
    .description("Settle a gateway-funded x402 round (any payment rail)")
    .requiredOption("--source-id <hex>", "32-byte source identifier")
    .requiredOption("--canonical-timestamp <number>", "canonical timestamp")
    .requiredOption("--signatures-required <number>", "quorum threshold")
    .requiredOption("--registry-version <number>", "registry version")
    .requiredOption("--amount <amount>", "exact x402 round price")
    .requiredOption(
      "--payer-id <hex>",
      "32-byte rail-scoped payer identity (keccak256(rail_id_be || payer_address))"
    )
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet"
    )
    .option(
      "--payment-rail <number>",
      "payment rail registry id (0 = none, 1 = solana, 2 = base)",
      "1"
    )
    .option(
      "--payment-ref <hex>",
      "opaque payment reference (tx hash / facilitator receipt), <= 64 bytes"
    )
    .option(
      "--gateway-usdc <pubkey>",
      "gateway float token account; defaults to the gateway authority's own USDC ATA"
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

      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey
      );
      const gatewayUsdc = options.gatewayUsdc
        ? parsePublicKey(options.gatewayUsdc, "gateway USDC account")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            gatewayAuthority.publicKey
          );

      const payerId = hexToBytes(options.payerId, 32, "payer id");
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
      const paymentRail = parseInteger(options.paymentRail, "payment rail");
      const paymentRef = options.paymentRef
        ? hexToBytes(options.paymentRef, undefined, "payment ref")
        : new Uint8Array(0);
      if (paymentRef.length > 64) {
        throw new Error(
          `payment ref must be <= 64 bytes, got ${paymentRef.length}`
        );
      }
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const amount = bn(options.amount, "amount");
      const value = hexToBytes(options.value, 32, "value");
      const aggSig = options.aggSig
        ? parseAggregatedSigJson(options.aggSig)
        : buildAggregatedSig({
            signersBitmap: signersBitmapFromOptions(options),
          });
      const creditedBits = creditedBitsFromBitmap(
        Uint8Array.from(aggSig.signersBitmap)
      );
      const [roundRecordPda] = findX402RoundPda(
        ctx.programId,
        payerId,
        gatewayPda,
        sourceId,
        signaturesRequired,
        canonicalTimestamp
      );

      const signers = [];
      if (!gatewayAuthority.publicKey.equals(ctx.wallet.publicKey)) {
        signers.push(gatewayAuthority);
      }

      const signature = await (ctx.program.methods as any)
        .settleX402Round({
          payerId: Array.from(payerId),
          amount,
          sourceId: Array.from(sourceId),
          canonicalTimestamp: new BN(canonicalTimestamp),
          signaturesRequired,
          registryVersion,
          aggSig,
          value: Array.from(value),
          paymentRail,
          paymentRef: Buffer.from(paymentRef),
        })
        .accountsPartial({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          registry: registryPda,
          gateway: gatewayPda,
          usdcMint: protocolConfig.usdcMint,
          gatewayUsdc,
          treasury: protocolConfig.treasury,
          tokenProgram: TOKEN_PROGRAM_ID,
          roundRecord: roundRecordPda,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(
          creditedBits.map((bit) => ({
            pubkey: registryNode(registry, bit),
            isWritable: false,
            isSigner: false,
          }))
        )
        .signers(signers)
        .rpc();

      writeJson({
        signature,
        roundRecord: roundRecordPda.toBase58(),
        payerId: `0x${Buffer.from(payerId).toString("hex")}`,
        paymentRail,
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
    .option("--challenger-usdc <pubkey>", "challenger USDC ATA")
    .option(
      "--subscription <pubkey>",
      "subscription account (required for subscription rounds; omit for x402 rounds — they are never refunded)"
    )
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
      const gatewayBond = findGatewayBondAta(
        protocolConfig.usdcMint,
        gatewayPda
      );
      const challengerUsdc = options.challengerUsdc
        ? parsePublicKey(options.challengerUsdc, "challenger USDC")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );

      // `record.kind` discriminates the funding path: subscription rounds are restored on a
      // successful dispute; x402 rounds are never refunded (the price came from the gateway's
      // own float, and the gateway is the party being slashed).
      const isSubscription = record.kind?.subscription !== undefined;
      const subscription = isSubscription
        ? parsePublicKey(
            requireValue(options.subscription, "Missing --subscription"),
            "subscription"
          )
        : null;

      const signature = await (ctx.program.methods as any)
        .disputeRound()
        .accountsPartial({
          challenger: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          registry: registryPda,
          roundRecord,
          gateway: gatewayPda,
          gatewayBond,
          treasury: protocolConfig.treasury,
          challengerUsdc,
          tokenProgram: TOKEN_PROGRAM_ID,
          subscription,
        })
        .remainingAccounts(
          creditedBitsFromBitmap(Uint8Array.from(record.signersBitmap)).map(
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
