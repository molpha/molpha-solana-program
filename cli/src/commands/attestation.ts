import { keccak_256 } from "@noble/hashes/sha3.js";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { ComputeBudgetProgram } from "@solana/web3.js";
import { Command } from "commander";
import { accountJson, fetchAccountOrNull } from "../lib/accounts";
import { getCliContext, getGlobalOptions } from "../lib/client";
import { resolveCliOptions } from "../lib/config";
import { findFeedPda, findRegistryPda } from "../lib/pdas";
import {
  buildAttestation,
  buildBitmapFromFlag,
  buildSubmitArgsFromFixture,
  describeSignedPayload,
  generateE2eSignerMaterialFromSeed,
  rawValueArg,
  signE2eAttestation,
  signerRemainingAccounts,
  SubmitAttestationArgs,
} from "../lib/signing/attestation";
import { loadVerifyFixture } from "../lib/signing/fixtures";
import {
  hexToBytes,
  parseInteger,
  readJsonFile,
  requireValue,
  writeJson,
} from "../lib/utils";

type ByteField = number[] | string;

/** `--args` file: `SubmitAttestationArgs`, as printed under `submitArgs` by `attestation sign`. */
type SubmitArgsShape = {
  attestation: {
    payload: {
      value: ByteField;
      sourceId: ByteField;
      registryVersion: number | string;
      signaturesRequired: number | string;
      canonicalTimestamp: number | string;
    };
    signature: {
      aggSigS: ByteField;
      commitment: ByteField;
      signersBitmap: ByteField;
    };
  };
  rawValue?: ByteField | null;
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
    .option("--value <hex>", "32-byte oracle value (hex)")
    .option(
      "--raw-value <hex>",
      "value preimage (hex, <= 256 bytes); signs keccak256(raw) instead of --value",
    )
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
    .option("--value <hex>", "32-byte oracle value (hex)")
    .option(
      "--raw-value <hex>",
      "value preimage (hex, <= 256 bytes); signs keccak256(raw) instead of --value",
    )
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
      const { payload, signature: schnorrSignature } = submitArgs.attestation;
      const [feedPda] = findFeedPda(
        ctx.programId,
        Uint8Array.from(payload.sourceId),
        payload.signaturesRequired,
        ctx.wallet.publicKey,
      );
      const registryVersion = payload.registryVersion;
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);
      const remaining = signerRemainingAccounts(
        registry,
        schnorrSignature.signersBitmap,
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
}

function buildSignedPayload(
  options: Record<string, string | undefined>,
  programId: PublicKey,
) {
  if (options.fixture) {
    const fixture = loadVerifyFixture(options.fixture);
    const submitArgs = buildSubmitArgsFromFixture(fixture);
    const { signature } = submitArgs.attestation;
    const signersBitmap = Uint8Array.from(signature.signersBitmap);
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
        aggSigS: Uint8Array.from(signature.aggSigS),
        commitment: Uint8Array.from(signature.commitment),
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
  if (options.value && options.rawValue) {
    throw new Error("Pass either --value or --raw-value, not both");
  }
  const rawValue = options.rawValue
    ? hexToBytes(options.rawValue, undefined, "raw value")
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
      value: rawValue
        ? keccak_256(rawValue)
        : hexToBytes(
            requireValue(options.value, "Missing --value or --raw-value"),
            32,
            "value",
          ),
      rawValue,
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
): SubmitAttestationArgs {
  if (options.args) {
    return normalizeSubmitArgs(readJsonFile<SubmitArgsShape>(options.args));
  }
  if (options.fixture) {
    return buildSubmitArgsFromFixture(loadVerifyFixture(options.fixture));
  }
  return buildSignedPayload(options, programId).payload.submitArgs;
}

function normalizeSubmitArgs(value: SubmitArgsShape): SubmitAttestationArgs {
  if (!value.attestation?.payload || !value.attestation.signature) {
    throw new Error(
      "args file must contain attestation.payload and .signature",
    );
  }
  const { payload, signature } = value.attestation;

  return {
    attestation: buildAttestation({
      value: bytesFromField(payload.value, 32, "value"),
      sourceId: bytesFromField(payload.sourceId, 32, "source id"),
      registryVersion: parseInteger(
        String(payload.registryVersion),
        "registry version",
      ),
      signaturesRequired: parseInteger(
        String(payload.signaturesRequired),
        "signatures required",
      ),
      canonicalTimestamp: parseInteger(
        String(payload.canonicalTimestamp),
        "canonical timestamp",
      ),
      aggSigS: bytesFromField(signature.aggSigS, 32, "agg sig s"),
      commitment: bytesFromField(signature.commitment, 20, "commitment"),
      signersBitmap: bytesFromField(
        signature.signersBitmap,
        32,
        "signers bitmap",
      ),
    }),
    rawValue: rawValueArg(
      value.rawValue
        ? bytesFromField(value.rawValue, undefined, "raw value")
        : undefined,
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
