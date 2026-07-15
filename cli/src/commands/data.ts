import { BN } from "@coral-xyz/anchor";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import { Command } from "commander";
import {
  accountJson,
  decodeVerifyReturnData,
  fetchAccountOrNull,
  resolveTransitionForRegistryVersion,
} from "../lib/accounts";
import { getCliContext, getGlobalOptions } from "../lib/client";
import { resolveCliOptions } from "../lib/config";
import {
  findFeedPda,
  findDelegatePda,
  findJobPda,
  findPlanPda,
  findRegistryStatePda,
  findSubscriptionPda,
} from "../lib/pdas";
import {
  buildBitmapFromFlag,
  buildSubmitArgsFromFixture,
  describeSignedPayload,
  generateE2eSignerMaterialFromSeed,
  signE2eDataUpdate,
  signerRemainingAccounts,
} from "../lib/signing/data-update";
import { loadVerifyFixture } from "../lib/signing/fixtures";
import {
  hexToBytes,
  parseInteger,
  readJsonFile,
  requireValue,
  writeJson,
} from "../lib/utils";

type SubmitArgsShape = {
  feedId: number[] | string;
  apiConfigHash?: number[] | string;
  configHash?: number[] | string;
  registryVersion: number | string;
  signaturesRequired: number | string;
  signersBitmap: number[] | string;
  value: number[] | string;
  canonicalTimestamp: string | number;
  aggSigR: number[] | string;
  aggSigS: number[] | string;
  commitmentAddr: number[] | string;
};

export function registerDataCommands(program: Command): void {
  const data = program
    .command("data")
    .description("Sign and submit data updates");

  data
    .command("sign")
    .description("Produce signed SubmitDataUpdate args")
    .option("--fixture <path>", "fixture file")
    .option("--feed-id <hex>", "32-byte feed id")
    .option("--registry-version <number>", "registry version")
    .option("--config-hash <hex>", "32-byte config hash")
    .option("--value <hex>", "32-byte value")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option("--decimals <number>", "feed decimals for lazy job init", "8")
    .option("--job-owner <pubkey>", "job owner for lazy job init")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers"
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
    .description("Submit a signed data update")
    .option("--fixture <path>", "fixture file")
    .option("--args <path>", "json file with submit args")
    .option("--feed-id <hex>", "32-byte feed id")
    .option("--registry-version <number>", "registry version")
    .option("--config-hash <hex>", "32-byte config hash")
    .option("--value <hex>", "32-byte value")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers"
    )
    .option("--compute-unit-limit <number>", "compute unit limit", "1400000")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      const submitArgs = resolveSubmitArgs(options, ctx.programId);
      const feedId = bytesFromField(submitArgs.feedId, 32, "feed id");
      const [jobPda] = findJobPda(ctx.programId, feedId);
      const [feedPda] = findFeedPda(ctx.programId, feedId);
      const existingJob = await fetchAccountOrNull<any>(
        ctx.program,
        "job",
        jobPda
      );
      const jobOwner = resolveJobOwner(
        options,
        existingJob,
        ctx.wallet.publicKey
      );
      const [subscriptionPda] = findSubscriptionPda(ctx.programId, jobOwner);
      const subscription = await fetchAccountOrNull<any>(
        ctx.program,
        "subscription",
        subscriptionPda
      );
      const planPda = subscription
        ? findPlanPda(ctx.programId, planTypeId(subscription.planType))[0]
        : null;
      const jobInit = resolveJobInitArgs(options, existingJob);
      if (jobInit && !subscription) {
        throw new Error(
          `No subscription found for lazy job init owner ${jobOwner.toBase58()}`
        );
      }
      const { remainingTransition } = await resolveTransitionForRegistryVersion(
        ctx.program,
        registryState,
        Number(submitArgs.registryVersion)
      );
      const remaining = signerRemainingAccounts(
        ctx.programId,
        bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap"),
        remainingTransition
      );

      const signature = await (ctx.program.methods as any)
        .submitDataUpdate(submitArgs, jobInit)
        .accountsPartial({
          submitter: ctx.wallet.publicKey,
          jobOwner: jobInit ? jobOwner : null,
          plan: jobInit ? planPda : null,
          subscription: jobInit ? subscriptionPda : null,
          delegate:
            jobInit && !jobOwner.equals(ctx.wallet.publicKey)
              ? findDelegatePda(
                  ctx.programId,
                  jobOwner,
                  ctx.wallet.publicKey
                )[0]
              : null,
          registryState: registryStatePda,
          job: jobPda,
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
        job: jobPda.toBase58(),
        feed: feedPda.toBase58(),
        feedAccount: feed ? accountJson(feedPda, feed) : null,
      });
    });

  data
    .command("verify")
    .description("Simulate verify_data_update and print return data")
    .option("--fixture <path>", "fixture file")
    .option("--args <path>", "json file with submit args")
    .option("--feed-id <hex>", "32-byte feed id")
    .option("--registry-version <number>", "registry version")
    .option("--config-hash <hex>", "32-byte config hash")
    .option("--value <hex>", "32-byte value")
    .option("--canonical-timestamp <number>", "unix timestamp")
    .option("--signer-count <number>", "number of signers to include", "8")
    .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
    .option("--signatures-required <number>", "override signatures required")
    .option(
      "--signer-seed <string>",
      "deterministic signer seed",
      "molpha-cli-e2e-signers"
    )
    .option("--compute-unit-limit <number>", "compute unit limit", "1400000")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      const submitArgs = resolveSubmitArgs(options, ctx.programId);
      const [jobPda] = findJobPda(
        ctx.programId,
        bytesFromField(submitArgs.feedId, 32, "feed id")
      );
      const { remainingTransition } = await resolveTransitionForRegistryVersion(
        ctx.program,
        registryState,
        Number(submitArgs.registryVersion)
      );
      const remaining = signerRemainingAccounts(
        ctx.programId,
        bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap"),
        remainingTransition
      );

      const verifyIx = await (ctx.program.methods as any)
        .verifyDataUpdate(submitArgs)
        .accountsPartial({
          registryState: registryStatePda,
        })
        .remainingAccounts(remaining)
        .instruction();

      const tx = new Transaction().add(
        ComputeBudgetProgram.setComputeUnitLimit({
          units: parseInteger(options.computeUnitLimit, "compute unit limit"),
        }),
        verifyIx
      );
      tx.feePayer = ctx.wallet.publicKey;
      tx.recentBlockhash = (
        await ctx.connection.getLatestBlockhash(ctx.options.commitment)
      ).blockhash;
      tx.sign(ctx.payer);

      const simulation = await ctx.provider.simulate(
        tx,
        [ctx.payer],
        ctx.options.commitment
      );
      if (!simulation.returnData) {
        throw new Error("verify_data_update returned no return data");
      }

      const rawBytes = Buffer.from(simulation.returnData.data[0], "base64");
      writeJson({
        job: jobPda.toBase58(),
        returnData: decodeVerifyReturnData(rawBytes),
        logs: simulation.logs ?? [],
      });
    });
}

function buildSignedPayload(
  options: Record<string, string | undefined>,
  programId: ReturnType<typeof getProgramIdFromGlobals>
) {
  if (options.fixture) {
    const fixture = loadVerifyFixture(options.fixture);
    const submitArgs = buildSubmitArgsFromFixture(fixture);
    return {
      payload: {
        submitArgs,
        signersBitmap: bytesFromField(
          submitArgs.signersBitmap,
          32,
          "signers bitmap"
        ),
        aggSigS: bytesFromField(submitArgs.aggSigS, 32, "agg sig s"),
        commitmentAddr: bytesFromField(
          submitArgs.commitmentAddr,
          20,
          "commitment"
        ),
      },
      remaining: signerRemainingAccounts(
        programId,
        bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap")
      ),
    };
  }

  const signerCount = parseInteger(options.signerCount ?? "8", "signer count");
  const signerSeed = options.signerSeed ?? "molpha-cli-e2e-signers";
  const signerMaterial = generateE2eSignerMaterialFromSeed(
    signerSeed,
    signerCount
  );
  const signersBitmap = options.signersBitmap
    ? buildBitmapFromFlag(options.signersBitmap)
    : undefined;

  const payload = signE2eDataUpdate(
    parseInteger(
      requireValue(options.registryVersion, "Missing --registry-version"),
      "registry version"
    ),
    signerMaterial,
    signersBitmap ?? signerCount,
    hexToBytes(
      requireValue(options.feedId, "Missing --feed-id"),
      32,
      "feed id"
    ),
    {
      configHash: hexToBytes(
        requireValue(options.configHash, "Missing --config-hash"),
        32,
        "config hash"
      ),
      value: hexToBytes(
        requireValue(options.value, "Missing --value"),
        32,
        "value"
      ),
      canonicalTimestamp: parseInteger(
        requireValue(
          options.canonicalTimestamp,
          "Missing --canonical-timestamp"
        ),
        "canonical timestamp"
      ),
    },
    options.signaturesRequired
      ? parseInteger(options.signaturesRequired, "signatures required")
      : undefined
  );

  return {
    payload,
    remaining: signerRemainingAccounts(programId, payload.signersBitmap),
  };
}

function resolveSubmitArgs(
  options: Record<string, string | undefined>,
  programId: PublicKey
) {
  if (options.args) {
    return normalizeSubmitArgs(readJsonFile<SubmitArgsShape>(options.args));
  }
  if (options.fixture) {
    return buildSubmitArgsFromFixture(loadVerifyFixture(options.fixture));
  }
  return buildSignedPayload(options, programId).payload.submitArgs;
}

function resolveJobOwner(
  options: Record<string, string | undefined>,
  existingJob: any | null,
  defaultOwner: PublicKey
): PublicKey {
  if (options.jobOwner) {
    return new PublicKey(options.jobOwner);
  }
  if (existingJob?.owner) {
    return existingJob.owner;
  }
  return defaultOwner;
}

function resolveJobInitArgs(
  options: Record<string, string | undefined>,
  existingJob: any | null
) {
  if (existingJob) {
    return null;
  }

  let configHash: Uint8Array | null = null;
  if (options.configHash) {
    configHash = hexToBytes(options.configHash, 32, "config hash");
  } else if (options.fixture) {
    const fixture = loadVerifyFixture(options.fixture);
    configHash = hexToBytes(fixture.dataUpdate.configHash, 32, "config hash");
  } else if (options.args) {
    const raw = readJsonFile<SubmitArgsShape>(options.args);
    const rawConfigHash = raw.apiConfigHash ?? raw.configHash;
    if (rawConfigHash) {
      configHash = bytesFromField(rawConfigHash, 32, "config hash");
    }
  }

  if (!configHash) {
    throw new Error(
      "Missing --config-hash for lazy job init; pass --config-hash or submit after the job exists"
    );
  }

  return {
    apiConfigHash: Array.from(configHash),
    decimals: parseInteger(options.decimals ?? "8", "decimals"),
  };
}

function planTypeId(planType: Record<string, object> | number): number {
  if (typeof planType === "number") {
    return planType;
  }
  if ("basic" in planType) return 0;
  if ("standard" in planType) return 1;
  if ("professional" in planType) return 2;
  if ("enterprise" in planType) return 3;
  throw new Error(`Unknown plan type: ${JSON.stringify(planType)}`);
}

function normalizeSubmitArgs(value: SubmitArgsShape) {
  return {
    feedId: Array.from(bytesFromField(value.feedId, 32, "feed id")),
    registryVersion:
      typeof value.registryVersion === "number"
        ? value.registryVersion
        : parseInteger(value.registryVersion, "registry version"),
    signaturesRequired:
      typeof value.signaturesRequired === "number"
        ? value.signaturesRequired
        : parseInteger(value.signaturesRequired, "signatures required"),
    signersBitmap: Array.from(
      bytesFromField(value.signersBitmap, 32, "signers bitmap")
    ),
    value: Array.from(bytesFromField(value.value, 32, "value")),
    canonicalTimestamp: new BN(String(value.canonicalTimestamp)),
    aggSigR: Array.from(bytesFromField(value.aggSigR, 32, "agg sig r")),
    aggSigS: Array.from(bytesFromField(value.aggSigS, 32, "agg sig s")),
    commitmentAddr: Array.from(
      bytesFromField(value.commitmentAddr, 20, "commitment")
    ),
  };
}

function bytesFromField(
  value: string | number[] | Uint8Array,
  expectedLength: number,
  label: string
): Uint8Array {
  if (typeof value === "string") {
    return hexToBytes(value, expectedLength, label);
  }
  const bytes = Uint8Array.from(value);
  if (bytes.length !== expectedLength) {
    throw new Error(
      `${label} must be ${expectedLength} bytes, got ${bytes.length}`
    );
  }
  return bytes;
}

function getProgramIdFromGlobals(command: Command) {
  const resolved = resolveCliOptions(getGlobalOptions(command));
  return new PublicKey(resolved.programId);
}
