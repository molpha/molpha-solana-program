#!/usr/bin/env node

/**
 * End-to-end devnet/localnet flow using registered nodes from generated-nodes.json:
 *   1. subscribe (monthly plan)
 *   2. create_job + feed
 *   3. sign coalition Schnorr update with manifest EVM keys
 *   4. submit_data_update
 *
 * Prerequisites:
 *   - anchor build -p molpha
 *   - Protocol initialized with an active plan matching --plan-id
 *   - Nodes registered (see scripts/generate-and-register-nodes.js)
 *   - Wallet funded with SOL + USDC for subscription
 *
 * Usage:
 *   node scripts/subscribe-create-job-submit.js [options]
 *
 * Options:
 *   --cluster <name>           localnet | devnet | mainnet
 *   --rpc <url>
 *   --wallet <path>
 *   --nodes <path>             generated-nodes.json (default: ./generated-nodes.json)
 *   --plan-id <n>              0=basic, 1=standard, 2=professional, 3=enterprise (default: 1)
 *   --signatures-required <n>  coalition size (default: min(3, registered node count))
 *   --decimals <n>             feed decimals (default: 8)
 *   --seed <hex>               deterministic payload seed (default: random)
 *   --config-hash <hex>        32-byte api config hash (overrides seed)
 *   --value <hex>              32-byte feed value (overrides seed)
 *   --canonical-timestamp <n>  unix seconds (default: now)
 *   --output <path>            write run summary JSON (default: ./job-submit-result.json)
 *   --skip-subscribe
 *   --skip-create-job
 *   --skip-submit
 *   --compute-unit-limit <n>   default: 1400000
 */

const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const anchor = require("@coral-xyz/anchor");
const { BN } = require("@coral-xyz/anchor");
const {
  PublicKey,
  SystemProgram,
  ComputeBudgetProgram,
} = require("@solana/web3.js");
const { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } = require("@solana/spl-token");
const { keccak_256 } = require("@noble/hashes/sha3");
const { createProvider } = require("./lib/molpha-provider");
const {
  deriveJobId,
  registryIndexRemainingAccounts,
  signerMaterialFromManifestNodes,
  signDataUpdateForRegistry,
} = require("./lib/cu-bench-data-update-common");

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");

const DEFAULT_NODES = path.resolve(process.cwd(), "generated-nodes.json");
const DEFAULT_OUTPUT = path.resolve(process.cwd(), "job-submit-result.json");
const PLAN_NAMES = ["basic", "standard", "professional", "enterprise"];

function parseArgs(argv) {
  const opts = {
    cluster: null,
    rpc: null,
    wallet: null,
    nodes: DEFAULT_NODES,
    planId: 1,
    signaturesRequired: null,
    decimals: 8,
    seed: null,
    configHash: null,
    value: null,
    canonicalTimestamp: null,
    output: DEFAULT_OUTPUT,
    skipSubscribe: false,
    skipCreateJob: false,
    skipSubmit: false,
    computeUnitLimit: 1_400_000,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--cluster" || arg === "-c") opts.cluster = argv[++i];
    else if (arg === "--rpc") opts.rpc = argv[++i];
    else if (arg === "--wallet" || arg === "-w") opts.wallet = argv[++i];
    else if (arg === "--nodes" || arg === "-n") opts.nodes = path.resolve(argv[++i]);
    else if (arg === "--plan-id") opts.planId = parseInt(argv[++i], 10);
    else if (arg === "--signatures-required") opts.signaturesRequired = parseInt(argv[++i], 10);
    else if (arg === "--decimals") opts.decimals = parseInt(argv[++i], 10);
    else if (arg === "--seed") opts.seed = argv[++i];
    else if (arg === "--config-hash") opts.configHash = argv[++i];
    else if (arg === "--value") opts.value = argv[++i];
    else if (arg === "--canonical-timestamp") opts.canonicalTimestamp = parseInt(argv[++i], 10);
    else if (arg === "--output" || arg === "-o") opts.output = path.resolve(argv[++i]);
    else if (arg === "--skip-subscribe") opts.skipSubscribe = true;
    else if (arg === "--skip-create-job") opts.skipCreateJob = true;
    else if (arg === "--skip-submit") opts.skipSubmit = true;
    else if (arg === "--compute-unit-limit") opts.computeUnitLimit = parseInt(argv[++i], 10);
    else if (arg === "--help" || arg === "-h") {
      console.log(fs.readFileSync(__filename, "utf8").split("\n").slice(0, 35).join("\n"));
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (!Number.isFinite(opts.planId) || opts.planId < 0 || opts.planId > 3) {
    throw new Error("--plan-id must be 0-3");
  }
  return opts;
}

function planTypeToAnchor(planId) {
  const name = PLAN_NAMES[planId];
  if (!name) {
    throw new Error(`Invalid plan id ${planId}`);
  }
  return { [name]: {} };
}

function planTypeId(planType) {
  if (planType.basic !== undefined) return 0;
  if (planType.standard !== undefined) return 1;
  if (planType.professional !== undefined) return 2;
  if (planType.enterprise !== undefined) return 3;
  if (typeof planType === "number") return planType;
  throw new Error(`Unknown plan type: ${JSON.stringify(planType)}`);
}

function hexToBuffer32(hex, label) {
  const clean = hex.replace(/^0x/, "");
  if (clean.length !== 64) {
    throw new Error(`${label} must be 32 bytes (64 hex chars)`);
  }
  return Buffer.from(clean, "hex");
}

function labeledBytes32(seed, label) {
  return Buffer.from(keccak_256(Buffer.concat([Buffer.from(seed), Buffer.from(label)])));
}

function buildPayload(opts) {
  const seed = opts.seed ?? crypto.randomBytes(16).toString("hex");
  const configHash = opts.configHash
    ? hexToBuffer32(opts.configHash, "config hash")
    : labeledBytes32(seed, "apiConfigHash");
  const value = opts.value ? hexToBuffer32(opts.value, "value") : labeledBytes32(seed, "feedValue");
  const canonicalTimestamp =
    opts.canonicalTimestamp ?? Math.floor(Date.now() / 1000);
  return { seed, configHash, value, canonicalTimestamp };
}

function loadNodesManifest(filepath) {
  const raw = JSON.parse(fs.readFileSync(filepath, "utf8"));
  if (!Array.isArray(raw.nodes) || raw.nodes.length === 0) {
    throw new Error(`Expected non-empty "nodes" in ${filepath}`);
  }
  return raw;
}

function writeJson(filepath, data) {
  fs.mkdirSync(path.dirname(filepath), { recursive: true });
  fs.writeFileSync(filepath, JSON.stringify(data, null, 2), "utf8");
}

async function fetchOptionalAccount(program, name, pubkey) {
  try {
    return await program.account[name].fetch(pubkey);
  } catch {
    return null;
  }
}

async function main() {
  const opts = parseArgs(process.argv);
  const manifest = loadNodesManifest(opts.nodes);
  const { program, programId } = createProvider({
    cluster: opts.cluster,
    rpc: opts.rpc,
    wallet: opts.wallet,
    programId: manifest.programId,
  });
  const provider = anchor.getProvider();
  const owner = provider.wallet.publicKey;

  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const [planPda] = PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([opts.planId])],
    programId
  );
  const [subscriptionPda] = PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, owner.toBuffer()],
    programId
  );

  const protocolConfig = await program.account.protocolConfig.fetch(protocolConfigPda);
  const registryState = await program.account.registryState.fetch(registryStatePda);
  const plan = await fetchOptionalAccount(program, "plan", planPda);
  if (!plan) {
    throw new Error(
      `Plan ${PLAN_NAMES[opts.planId]} (id ${opts.planId}) not found at ${planPda.toBase58()}. Add it first (molpha plan add).`
    );
  }
  if (!plan.isActive) {
    throw new Error(`Plan ${planPda.toBase58()} is inactive`);
  }

  const nodeCount = Number(registryState.currentNodeCount);
  if (nodeCount === 0) {
    throw new Error("Registry has no nodes; run generate-and-register-nodes.js first");
  }

  const signaturesRequired =
    opts.signaturesRequired ?? Math.min(3, nodeCount);
  if (signaturesRequired < 1 || signaturesRequired > nodeCount) {
    throw new Error(
      `--signatures-required must be between 1 and registered node count (${nodeCount})`
    );
  }
  if (signaturesRequired > plan.maxSigners) {
    throw new Error(
      `signatures required ${signaturesRequired} exceeds plan max_signers ${plan.maxSigners}`
    );
  }

  const signerMaterial = signerMaterialFromManifestNodes(manifest.nodes, nodeCount);
  const payload = buildPayload(opts);
  const jobId = deriveJobId(owner, payload.configHash);
  const [jobPda] = PublicKey.findProgramAddressSync([JOB_SEED, jobId], programId);
  const [feedPda] = PublicKey.findProgramAddressSync([FEED_SEED, jobId], programId);

  const ownerUsdc = getAssociatedTokenAddressSync(protocolConfig.usdcMint, owner);

  console.log(`Program: ${programId.toBase58()}`);
  console.log(`Owner: ${owner.toBase58()}`);
  console.log(`Registry nodes: ${nodeCount} (version ${registryState.currentVersion})`);
  console.log(`Plan: ${PLAN_NAMES[opts.planId]} @ ${planPda.toBase58()}`);
  console.log(`Signatures required: ${signaturesRequired}`);
  console.log(`Job id: 0x${jobId.toString("hex")}`);
  console.log("");

  const result = {
    ranAt: new Date().toISOString(),
    cluster: opts.cluster ?? manifest.cluster,
    programId: programId.toBase58(),
    owner: owner.toBase58(),
    nodesManifest: opts.nodes,
    planId: opts.planId,
    registryVersion: registryState.currentVersion,
    nodeCount,
    signaturesRequired,
    seed: payload.seed,
    configHash: `0x${payload.configHash.toString("hex")}`,
    value: `0x${payload.value.toString("hex")}`,
    canonicalTimestamp: payload.canonicalTimestamp,
    jobId: `0x${jobId.toString("hex")}`,
    job: jobPda.toBase58(),
    feed: feedPda.toBase58(),
    steps: {},
  };

  if (!opts.skipSubscribe) {
    const existingSub = await fetchOptionalAccount(program, "subscription", subscriptionPda);
    if (existingSub && existingSub.validUntil > Math.floor(Date.now() / 1000)) {
      console.log(`⏭️  Active subscription already exists until ${existingSub.validUntil}`);
      result.steps.subscribe = { skipped: true, subscription: subscriptionPda.toBase58() };
    } else {
      console.log("📋 Subscribing...");
      const sig = await program.methods
        .subscribe(planTypeToAnchor(opts.planId))
        .accountsPartial({
          owner,
          protocolConfig: protocolConfigPda,
          plan: planPda,
          subscription: subscriptionPda,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
      console.log(`   ✅ subscribe: ${sig}`);
      result.steps.subscribe = { signature: sig, subscription: subscriptionPda.toBase58() };
    }
  }

  const subscription = await program.account.subscription.fetch(subscriptionPda);
  const nowSecs = Math.floor(Date.now() / 1000);
  if (Number(subscription.validUntil) <= nowSecs) {
    throw new Error("Subscription is missing or expired; run without --skip-subscribe");
  }

  const subscribedPlanId = planTypeId(subscription.planType);
  const [subscribedPlanPda] = PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([subscribedPlanId])],
    programId
  );

  if (!opts.skipCreateJob) {
    const existingJob = await fetchOptionalAccount(program, "job", jobPda);
    if (existingJob) {
      console.log(`⏭️  Job already exists at ${jobPda.toBase58()}`);
      result.steps.createJob = { skipped: true, job: jobPda.toBase58(), feed: feedPda.toBase58() };
    } else {
      console.log("🛠️  Creating job + feed...");
      const sig = await program.methods
        .createJob(
          {
            apiConfigHash: Array.from(payload.configHash),
            signaturesRequired,
            decimals: opts.decimals,
          },
          Array.from(jobId)
        )
        .accountsPartial({
          owner,
          protocolConfig: protocolConfigPda,
          plan: subscribedPlanPda,
          job: jobPda,
          subscription: subscriptionPda,
          feed: feedPda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      console.log(`   ✅ create_job: ${sig}`);
      result.steps.createJob = {
        signature: sig,
        job: jobPda.toBase58(),
        feed: feedPda.toBase58(),
      };
    }
  }

  if (!opts.skipSubmit) {
    const feedBefore = await program.account.feed.fetch(feedPda);
    const ts =
      payload.canonicalTimestamp > Number(feedBefore.canonicalTimestamp)
        ? payload.canonicalTimestamp
        : Number(feedBefore.canonicalTimestamp) + 1;

    const registryVersion = Number(registryState.currentVersion);
    const signed = signDataUpdateForRegistry({
      registryVersion,
      signerMaterial,
      signaturesRequired,
      jobId,
      value: payload.value,
      canonicalTimestamp: ts,
      nodeCount,
      redundancyBuffer: Number(registryState.redundancyBuffer ?? 2),
    });

    const remaining = registryIndexRemainingAccounts(
      programId,
      signed.signersBitmap
    );

    console.log("📤 Submitting data update...");
    console.log(`   signers bitmap: 0x${Buffer.from(signed.signersBitmap).toString("hex")}`);
    console.log(`   canonical timestamp: ${ts}`);

    const sig = await program.methods
      .submitDataUpdate(signed.submitArgs)
      .accountsPartial({
        submitter: owner,
        registryState: registryStatePda,
        feed: feedPda,
      })
      .remainingAccounts(remaining)
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: opts.computeUnitLimit }),
      ])
      .rpc();

    const feedAfter = await program.account.feed.fetch(feedPda);
    console.log(`   ✅ submit_data_update: ${sig}`);
    console.log(`   feed value: 0x${Buffer.from(feedAfter.value).toString("hex")}`);

    result.steps.submit = {
      signature: sig,
      canonicalTimestamp: ts,
      signersBitmap: `0x${Buffer.from(signed.signersBitmap).toString("hex")}`,
      feedValue: `0x${Buffer.from(feedAfter.value).toString("hex")}`,
    };
    result.submitArgs = signed.submitArgs;
  }

  writeJson(opts.output, result);
  console.log(`\n💾 Wrote summary to ${opts.output}`);
}

main().catch((err) => {
  console.error("❌", err.message || err);
  if (err.logs) {
    console.error(err.logs.join("\n"));
  }
  process.exit(1);
});
