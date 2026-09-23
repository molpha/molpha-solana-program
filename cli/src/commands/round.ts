import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  Ed25519Program,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import { Command } from "commander";
import {
  creditedBitsFromBitmap,
  parseSchnorrSignatureJson,
} from "../lib/agg-sig";
import { hashRequestAuthFields } from "../lib/auth";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { MAX_SETTLEMENT_DELAY_CAP_SECONDS } from "../lib/protocol-config";
import {
  buildAttestation,
  settlementRemainingAccounts,
} from "../lib/signing/attestation";
import {
  deriveRoundId,
  findDelegatePda,
  findGatewayPda,
  findProtocolConfigPda,
  findRegistryPda,
  findRoundReplayPda,
  findSubscriptionPda,
  findSubscriptionRoundPda,
  findTreasuryAta,
  findX402RoundPda,
  RoundReplayKind,
} from "../lib/pdas";
import {
  bn,
  hexToBytes,
  loadKeypair,
  parseInteger,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

export function registerRoundCommands(program: Command): void {
  const round = program
    .command("round")
    .description("Verify and settle rounds");

  round
    .command("settle-subscription")
    .description("Settle a subscription/delegate paid round")
    .requiredOption("--source-id <hex>", "32-byte source identifier")
    .requiredOption("--timestamp <number>", "canonical timestamp")
    .requiredOption("--signatures-required <number>", "quorum threshold")
    .requiredOption("--registry-version <number>", "registry version")
    .requiredOption(
      "--gateway-authority <pubkey>",
      "gateway authority pubkey (for PDA)",
    )
    .option(
      "--value <hex>",
      "32-byte oracle value (hex)",
      "0x" + "00".repeat(32),
    )
    .option("--subscription-owner <pubkey>", "subscription owner")
    .option(
      "--consumer-wallet <path>",
      "consumer (owner or delegate) keypair that signs RequestAuth",
    )
    .requiredOption("--agg-sig <path>", "JSON SchnorrSignature file")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda,
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
        subscriptionOwner,
      );
      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority",
      );
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);

      const sourceId = hexToBytes(options.sourceId, 32, "source id");
      const timestamp = parseInteger(options.timestamp, "timestamp");
      const signaturesRequired = parseInteger(
        options.signaturesRequired,
        "signatures required",
      );
      const registryVersion = parseInteger(
        options.registryVersion,
        "registry version",
      );
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const schnorrSignature = parseSchnorrSignatureJson(options.aggSig);
      const attestation = buildAttestation({
        value: hexToBytes(options.value, 32, "value"),
        sourceId,
        registryVersion,
        signaturesRequired,
        canonicalTimestamp: timestamp,
        ...schnorrSignature,
      });
      const creditedBits = creditedBitsFromBitmap(
        schnorrSignature.signersBitmap,
      );
      const remainingAccounts = settlementRemainingAccounts(
        registry,
        attestation.payload,
        schnorrSignature.signersBitmap,
      );
      // Replay markers are scoped to the signing consumer (owner or delegate).
      const [roundReplay] = findSubscriptionRoundPda(
        ctx.programId,
        consumer.publicKey,
        gatewayPda,
        sourceId,
        signaturesRequired,
        timestamp,
      );

      const consumerDelegate = consumer.publicKey.equals(subscriptionOwner)
        ? null
        : findDelegatePda(
            ctx.programId,
            subscriptionOwner,
            consumer.publicKey,
          )[0];

      const authHash = hashRequestAuthFields(
        ctx.programId,
        gatewayPda,
        sourceId,
        signaturesRequired,
        timestamp,
      );
      const signature = await (ctx.program.methods as any)
        .settleSubscriptionRound(attestation, consumer.publicKey)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          config: protocolConfigPda,
          registry: registryPda,
          subscription: subscriptionPda,
          treasury: findTreasuryAta(ctx.programId, protocolConfig.usdcMint),
          consumerDelegate,
          gateway: gatewayPda,
          roundReplay,
          instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .remainingAccounts(remainingAccounts)
        .preInstructions([
          Ed25519Program.createInstructionWithPrivateKey({
            privateKey: consumer.secretKey,
            message: authHash,
          }),
        ])
        .rpc();

      writeJson({
        signature,
        roundReplay: roundReplay.toBase58(),
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
      "32-byte rail-scoped payer identity (keccak256(rail_id_be || payer_address))",
    )
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet",
    )
    .option(
      "--payment-rail <number>",
      "payment rail registry id (0 = none, 1 = solana, 2 = base)",
      "1",
    )
    .option(
      "--payment-ref <hex>",
      "opaque payment reference (tx hash / facilitator receipt), <= 64 bytes",
    )
    .option(
      "--value <hex>",
      "32-byte oracle value (hex)",
      "0x" + "00".repeat(32),
    )
    .requiredOption("--agg-sig <path>", "JSON SchnorrSignature file")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda,
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey,
      );
      const gatewayUsdc = getAssociatedTokenAddressSync(
        protocolConfig.usdcMint,
        gatewayAuthority.publicKey,
      );

      const payerId = hexToBytes(options.payerId, 32, "payer id");
      const sourceId = hexToBytes(options.sourceId, 32, "source id");
      const canonicalTimestamp = parseInteger(
        options.canonicalTimestamp,
        "canonical timestamp",
      );
      const signaturesRequired = parseInteger(
        options.signaturesRequired,
        "signatures required",
      );
      const registryVersion = parseInteger(
        options.registryVersion,
        "registry version",
      );
      const paymentRail = parseInteger(options.paymentRail, "payment rail");
      const paymentRef = options.paymentRef
        ? hexToBytes(options.paymentRef, undefined, "payment ref")
        : new Uint8Array(0);
      if (paymentRef.length > 64) {
        throw new Error(
          `payment ref must be <= 64 bytes, got ${paymentRef.length}`,
        );
      }
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const amount = bn(options.amount, "amount");
      const schnorrSignature = parseSchnorrSignatureJson(options.aggSig);
      const attestation = buildAttestation({
        value: hexToBytes(options.value, 32, "value"),
        sourceId,
        registryVersion,
        signaturesRequired,
        canonicalTimestamp,
        ...schnorrSignature,
      });
      const creditedBits = creditedBitsFromBitmap(
        schnorrSignature.signersBitmap,
      );
      const remainingAccounts = settlementRemainingAccounts(
        registry,
        attestation.payload,
        schnorrSignature.signersBitmap,
      );
      const [roundReplay] = findX402RoundPda(
        ctx.programId,
        payerId,
        gatewayPda,
        sourceId,
        signaturesRequired,
        canonicalTimestamp,
      );

      const signers = [];
      if (!gatewayAuthority.publicKey.equals(ctx.wallet.publicKey)) {
        signers.push(gatewayAuthority);
      }

      const signature = await (ctx.program.methods as any)
        .settleX402Round(attestation, {
          payerId: Array.from(payerId),
          amount,
          paymentRail,
          paymentRef: Buffer.from(paymentRef),
        })
        .accountsPartial({
          authority: gatewayAuthority.publicKey,
          config: protocolConfigPda,
          registry: registryPda,
          gateway: gatewayPda,
          gatewayUsdc,
          treasury: findTreasuryAta(ctx.programId, protocolConfig.usdcMint),
          roundReplay,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(remainingAccounts)
        .signers(signers)
        .rpc();

      writeJson({
        signature,
        roundReplay: roundReplay.toBase58(),
        payerId: `0x${Buffer.from(payerId).toString("hex")}`,
        paymentRail,
        sourceId: `0x${Buffer.from(sourceId).toString("hex")}`,
        creditedBits,
      });
    });

  round
    .command("close-replay")
    .description(
      "Close an expired settlement replay marker; rent goes to the gateway authority",
    )
    .requiredOption(
      "--gateway-authority <pubkey>",
      "authority of the gateway the round was settled through",
    )
    .option("--address <pubkey>", "replay marker address")
    .option(
      "--kind <kind>",
      "settlement path when deriving the marker: subscription | x402",
    )
    .option(
      "--initiator <value>",
      "signing consumer pubkey (subscription) or 32-byte payer id hex (x402)",
    )
    .option("--source-id <hex>", "32-byte source identifier")
    .option("--signatures-required <number>", "quorum threshold")
    .option("--canonical-timestamp <number>", "canonical timestamp")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority",
      );
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);
      const roundReplay = options.address
        ? parsePublicKey(options.address, "replay marker")
        : findRoundReplayPda(
            ctx.programId,
            gatewayPda,
            roundIdFromOptions(options),
          )[0];

      const marker = await fetchAccountOrNull<any>(
        ctx.program,
        "roundReplayMarker",
        roundReplay,
      );
      if (!marker) {
        throw new Error(`Replay marker ${roundReplay.toBase58()} not found`);
      }
      const closableAfter =
        BigInt(marker.timestamp.toString()) +
        BigInt(MAX_SETTLEMENT_DELAY_CAP_SECONDS);
      const slot = await ctx.connection.getSlot();
      const now = await ctx.connection.getBlockTime(slot);
      if (now !== null && BigInt(now) <= closableAfter) {
        throw new Error(
          `Replay marker is closable after unix time ${closableAfter} (cluster time ${now})`,
        );
      }

      const signature = await (ctx.program.methods as any)
        .closeRoundReplay()
        .accountsPartial({
          gateway: gatewayPda,
          rentReceiver: gatewayAuthority,
          roundReplay,
        })
        .rpc();

      writeJson({
        signature,
        roundReplay: roundReplay.toBase58(),
        roundId: `0x${Buffer.from(marker.roundId).toString("hex")}`,
        rentReceiver: gatewayAuthority.toBase58(),
      });
    });
}

/** Re-derive a replay marker's round id from `close-replay` options. */
export function roundIdFromOptions(options: {
  kind?: string;
  initiator?: string;
  sourceId?: string;
  signaturesRequired?: string;
  canonicalTimestamp?: string;
}): Uint8Array {
  const missing = [
    "kind",
    "initiator",
    "sourceId",
    "signaturesRequired",
    "canonicalTimestamp",
  ].filter((key) => (options as any)[key] === undefined);
  if (missing.length > 0) {
    throw new Error(
      `Pass --address, or all of --kind --initiator --source-id --signatures-required --canonical-timestamp (missing: ${missing.join(", ")})`,
    );
  }
  let kind: RoundReplayKind;
  let initiator: Uint8Array;
  if (options.kind === "subscription") {
    kind = RoundReplayKind.Subscription;
    initiator = parsePublicKey(options.initiator!, "initiator").toBytes();
  } else if (options.kind === "x402") {
    kind = RoundReplayKind.X402;
    initiator = hexToBytes(options.initiator!, 32, "initiator");
  } else {
    throw new Error("--kind must be subscription or x402");
  }
  return deriveRoundId(
    kind,
    hexToBytes(options.sourceId!, 32, "source id"),
    parseInteger(options.signaturesRequired!, "signatures required"),
    parseInteger(options.canonicalTimestamp!, "canonical timestamp"),
    initiator,
  );
}
