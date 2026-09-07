import { BN } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { ComputeBudgetProgram } from "@solana/web3.js";
import { Command } from "commander";
import { accountJson, fetchAccountOrNull } from "../lib/accounts";
import { getCliContext, getGlobalOptions } from "../lib/client";
import { resolveCliOptions } from "../lib/config";
import { findFeedPda, findRegistryPda } from "../lib/pdas";
import {
  buildBitmapFromFlag,
  buildSubmitArgsFromFixture,
  describeSignedPayload,
  generateE2eSignerMaterialFromSeed,
  signE2eAttestation,
  signerRemainingAccounts,
} from "../lib/signing/attestation";
import { loadVerifyFixture } from "../lib/signing/fixtures";
import {
  hexToBytes,
  parseInteger,
  readJsonFile,
  requireValue,
  writeJson,
} from "../lib/utils";

type SubmitArgsShape = {
  sourceId?: number[] | string;
  registryVersion: number | string;
  signaturesRequired: number | string;
  signersBitmap: number[] | string;
  value: number[] | string | Uint8Array;
  canonicalTimestamp: string | number;
  aggSigS: number[] | string;
  commitment: number[] | string;
};

export function registerAttestationCommands(program: Command): void {
  const data = program
    .command("attestation")
    .description("Sign and submit attestations");

  data
    .command("sign")
    .description("Produce signed SubmitAttestation args")
    .option("--fixture <path>", "fixture file")
    .option("--source-id <hex>", "32-byte source identifier")
    .option("--registry-version <number>", "registry version")
    .option(
      "--config-hash <hex>",
      "optional 32-byte config hash (fixture metadata)",
    )
    .option("--value <hex>", "oracle value bytes (hex)")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers",
    )
    .action(async (options, command: Command) => {
      const programId = getProgramIdFromGlobals(command);
      const { payload, remaining } = buildSignedPayload(options, programId);
      writeJson({
        ...describeSignedPayload(payload),
        remainingAccounts: remaining.map((entry) => entry.pubkey.toBase58()),
      });
    });

  data
    .command("submit")
    .description("Submit a signed attestation (lazy-inits the feed if needed)")
    .option("--fixture <path>", "fixture file")
    .option("--args <path>", "json file with submit args")
    .option("--source-id <hex>", "32-byte source identifier")
    .option("--registry-version <number>", "registry version")
    .option(
      "--config-hash <hex>",
      "optional 32-byte config hash (fixture metadata)",
    )
    .option("--value <hex>", "oracle value bytes (hex)")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers",
    )
    .option("--compute-unit-limit <number>", "compute unit limit", "1400000")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const submitArgs = resolveSubmitArgs(options, ctx.programId);
      const [feedPda] = feedPdaFromSubmitArgs(
        ctx.programId,
        submitArgs,
        ctx.wallet.publicKey,
      );
      const registryVersion = Number(submitArgs.registryVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const remaining = signerRemainingAccounts(
        registry,
        submitArgs.signersBitmap,
      );

      const signature = await (ctx.program.methods as any)
        .submitAttestation(submitArgs)
        .accountsPartial({
          submitter: ctx.wallet.publicKey,
          registry: registryPda,
          feed: feedPda,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(remaining)
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({
            units: parseInteger(options.computeUnitLimit, "compute unit limit"),
          }),
        ])
        .rpc();

      const feed = await fetchAccountOrNull<any>(ctx.program, "feed", feedPda);
      writeJson({
        signature,
        feed: feedPda.toBase58(),
        feedAccount: feed ? accountJson(feedPda, feed) : null,
      });
    });

  data
    .command("verify")
    .description("Verify an attestation against any historical registry")
    .option("--fixture <path>", "fixture file")
    .option("--args <path>", "json file with submit args")
    .option("--source-id <hex>", "32-byte source identifier")
    .option("--registry-version <number>", "registry version")
    .option(
      "--config-hash <hex>",
      "optional 32-byte config hash (fixture metadata)",
    )
    .option("--value <hex>", "oracle value bytes (hex)")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const submitArgs = resolveSubmitArgs(options, ctx.programId);
      const [feedPda] = feedPdaFromSubmitArgs(
        ctx.programId,
        submitArgs,
        ctx.wallet.publicKey,
      );
      const feed = await fetchAccountOrNull<any>(ctx.program, "feed", feedPda);
      const registryVersion = Number(submitArgs.registryVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const remaining = signerRemainingAccounts(
        registry,
        submitArgs.signersBitmap,
      );
      await (ctx.program.methods as any)
        .verifyAttestation(submitArgs)
        .accountsPartial({ registry: registryPda })
        .remainingAccounts(remaining)
        .simulate();

      writeJson({
        feed: feedPda.toBase58(),
        feedExists: feed !== null,
        submitArgs: {
          ...submitArgs,
          canonicalTimestamp: String(submitArgs.canonicalTimestamp),
        },
        registry: registryPda.toBase58(),
        verified: true,
      });
    });
}

function buildSignedPayload(
  options: Record<string, string | undefined>,
  programId: PublicKey,
) {
  if (options.fixture) {
    const fixture = loadVerifyFixture(options.fixture);
    const submitArgs = buildSubmitArgsFromFixture(fixture);
    const signersBitmap = bytesFromField(
      submitArgs.signersBitmap,
      32,
      "signers bitmap",
    );
    const registry = {
      nodeCount: fixture.nodePubkeys.length,
      nodes: fixture.nodePubkeys.map((pubkey) =>
        hexToBytes(pubkey, 33, "node pubkey"),
      ),
    };
    return {
      payload: {
        submitArgs,
        signersBitmap,
        aggSigS: bytesFromField(submitArgs.aggSigS, 32, "agg sig s"),
        commitment: bytesFromField(submitArgs.commitment, 20, "commitment"),
      },
      remaining: signerRemainingAccounts(registry, signersBitmap),
    };
  }

  const signerCount = parseInteger(options.signerCount ?? "8", "signer count");
  const signerSeed = options.signerSeed ?? "molpha-cli-e2e-signers";
  const signerMaterial = generateE2eSignerMaterialFromSeed(
    signerSeed,
    signerCount,
  );
  const signersBitmap = options.signersBitmap
    ? buildBitmapFromFlag(options.signersBitmap)
    : undefined;

  const payload = signE2eAttestation(
    parseInteger(
      requireValue(options.registryVersion, "Missing --registry-version"),
      "registry version",
    ),
    signerMaterial,
    signersBitmap ?? signerCount,
    hexToBytes(
      requireValue(options.sourceId, "Missing --source-id"),
      32,
      "source id",
    ),
    {
      value: hexToBytes(
        requireValue(options.value, "Missing --value"),
        undefined,
        "value",
      ),
      canonicalTimestamp: parseInteger(
        requireValue(
          options.canonicalTimestamp,
          "Missing --canonical-timestamp",
        ),
        "canonical timestamp",
      ),
    },
    options.signaturesRequired
      ? parseInteger(options.signaturesRequired, "signatures required")
      : undefined,
  );

  return {
    payload,
    remaining: signerRemainingAccounts(
      {
        nodeCount: signerMaterial.compressedPubkeys.length,
        nodes: signerMaterial.compressedPubkeys,
      },
      payload.signersBitmap,
    ),
  };
}

function resolveSubmitArgs(
  options: Record<string, string | undefined>,
  programId: PublicKey,
) {
  if (options.args) {
    return normalizeSubmitArgs(readJsonFile<SubmitArgsShape>(options.args));
  }
  if (options.fixture) {
    return buildSubmitArgsFromFixture(loadVerifyFixture(options.fixture));
  }
  return buildSignedPayload(options, programId).payload.submitArgs;
}

function feedPdaFromSubmitArgs(
  programId: PublicKey,
  submitArgs: { sourceId: number[]; signaturesRequired: number },
  submitter: PublicKey,
) {
  return findFeedPda(
    programId,
    bytesFromField(submitArgs.sourceId, 32, "source id"),
    submitArgs.signaturesRequired,
    submitter,
  );
}

function sourceIdFromArgs(value: SubmitArgsShape): number[] {
  const raw = value.sourceId;
  if (raw === undefined) {
    throw new Error("Missing source id");
  }
  return Array.from(bytesFromField(raw, 32, "source id"));
}

function normalizeSubmitArgs(value: SubmitArgsShape) {
  const valueBytes = bytesFromField(value.value, 32, "value");
  const canonicalTimestamp =
    typeof value.canonicalTimestamp === "number"
      ? value.canonicalTimestamp
      : parseInteger(value.canonicalTimestamp, "canonical timestamp");

  return {
    sourceId: sourceIdFromArgs(value),
    registryVersion:
      typeof value.registryVersion === "number"
        ? value.registryVersion
        : parseInteger(value.registryVersion, "registry version"),
    value: Array.from(valueBytes),
    canonicalTimestamp: new BN(String(canonicalTimestamp)),
    signaturesRequired:
      typeof value.signaturesRequired === "number"
        ? value.signaturesRequired
        : parseInteger(value.signaturesRequired, "signatures required"),
    aggSigS: Array.from(bytesFromField(value.aggSigS, 32, "agg sig s")),
    commitment: Array.from(bytesFromField(value.commitment, 20, "commitment")),
    signersBitmap: Array.from(
      bytesFromField(value.signersBitmap, 32, "signers bitmap"),
    ),
  };
}

function bytesFromField(
  value: string | number[] | Uint8Array,
  expectedLength: number | undefined,
  label: string,
): Uint8Array {
  if (typeof value === "string") {
    return hexToBytes(value, expectedLength, label);
  }
  const bytes = Uint8Array.from(value);
  if (expectedLength !== undefined && bytes.length !== expectedLength) {
    throw new Error(
      `${label} must be ${expectedLength} bytes, got ${bytes.length}`,
    );
  }
  return bytes;
}

function getProgramIdFromGlobals(command: Command) {
  const resolved = resolveCliOptions(getGlobalOptions(command));
  return new PublicKey(resolved.programId);
}
