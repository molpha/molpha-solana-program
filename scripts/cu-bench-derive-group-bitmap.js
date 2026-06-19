/* eslint-disable no-console */
/**
 * Bankrun CU probe for on-chain `derive_group_bitmap` (bitmap.rs).
 *
 * deriveCu = CU(bench_derive_group_bitmap) - CU(bench_noop) per case.
 *
 * Prereq:
 *   cargo build-sbf --manifest-path programs/molpha/Cargo.toml \
 *     --sbf-out-dir target/deploy -- --features cu-bench
 */
const crypto = require("crypto");
const {
  PublicKey,
  Keypair,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  ComputeBudgetProgram,
} = require("@solana/web3.js");
const { BankrunProvider, startAnchor } = require("anchor-bankrun");
const fs = require("fs");
const path = require("path");
const toml = require("toml");

const {
  NODE_COUNT,
  COMPUTE_UNIT_LIMIT,
  FIXTURE_JOB_ID,
  FIXTURE_REGISTRY_VERSION,
  deriveSelectionSeed,
  effectiveSelectionSize,
} = require("./lib/cu-bench-data-update-common");

const REDUNDANCY_BUFFER = 2;
const SIG_REQ_CASES = [3, 8, 18];
const EVM_REF_CASES = [
  { nodeCount: 8, groupSize: 3, seed: Buffer.alloc(32, 0x11) },
  { nodeCount: 10, groupSize: 7, seed: Buffer.alloc(32, 0x11) },
  { nodeCount: 16, groupSize: 5, seed: Buffer.alloc(32, 0x11) },
  { nodeCount: 32, groupSize: 20, seed: Buffer.alloc(32, 0x11) },
];

function loadProgramId() {
  const anchorToml = toml.parse(
    fs.readFileSync(path.join(__dirname, "..", "Anchor.toml"), "utf8")
  );
  return new PublicKey(anchorToml.programs.localnet.molpha);
}

function anchorDiscriminator(ixName) {
  return crypto
    .createHash("sha256")
    .update(`global:${ixName}`)
    .digest()
    .subarray(0, 8);
}

function encodeNoopIx() {
  const data = Buffer.alloc(8);
  anchorDiscriminator("bench_noop").copy(data, 0);
  return data;
}

function encodeDeriveIx(seed, nodeCount, groupSize) {
  const data = Buffer.alloc(8 + 32 + 4 + 4);
  anchorDiscriminator("bench_derive_group_bitmap").copy(data, 0);
  seed.copy(data, 8);
  data.writeUInt32LE(nodeCount, 40);
  data.writeUInt32LE(groupSize, 44);
  return data;
}

async function simulateIxCu(provider, payer, programId, data) {
  const ix = new TransactionInstruction({
    programId,
    keys: [],
    data,
  });
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const tx = new Transaction();
  tx.add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT })
  );
  tx.add(ix);
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  tx.partialSign(payer);
  const sim = await provider.context.banksClient.simulateTransaction(tx);
  if (sim.result !== null) {
    throw new Error(`sim failed: ${sim.result}`);
  }
  return Number(sim.meta.computeUnitsConsumed);
}

async function measureDeriveCu(
  provider,
  payer,
  programId,
  seed,
  nodeCount,
  groupSize
) {
  const noopCu = await simulateIxCu(provider, payer, programId, encodeNoopIx());
  const deriveIxCu = await simulateIxCu(
    provider,
    payer,
    programId,
    encodeDeriveIx(seed, nodeCount, groupSize)
  );
  return { deriveCu: deriveIxCu - noopCu, noopCu, deriveIxCu };
}

async function main() {
  const soPath = path.join(__dirname, "..", "target", "deploy", "molpha.so");
  if (!fs.existsSync(soPath)) {
    throw new Error(
      "missing target/deploy/molpha.so — run:\n" +
        "  cargo build-sbf --manifest-path programs/molpha/Cargo.toml " +
        "--sbf-out-dir target/deploy -- --features cu-bench"
    );
  }

  const programId = loadProgramId();
  const payer = Keypair.generate();
  const context = await startAnchor(
    ".",
    [{ name: "molpha", programId }],
    [
      {
        address: payer.publicKey,
        info: {
          lamports: 1e12,
          executable: false,
          owner: SystemProgram.programId,
          data: Buffer.alloc(0),
        },
      },
    ],
    BigInt(COMPUTE_UNIT_LIMIT)
  );
  const provider = new BankrunProvider(context);
  provider.wallet = { publicKey: payer.publicKey };

  const noopBaseline = await simulateIxCu(
    provider,
    payer,
    programId,
    encodeNoopIx()
  );
  console.log(`bench_noop baseline: ${noopBaseline} CU\n`);

  const rows = [];

  console.log(
    "=== production-shaped (job selection seed, NODE_COUNT=%d) ===\n",
    NODE_COUNT
  );
  for (const sigReq of SIG_REQ_CASES) {
    const groupSize = effectiveSelectionSize(
      sigReq,
      REDUNDANCY_BUFFER,
      NODE_COUNT
    );
    const seed = deriveSelectionSeed(
      FIXTURE_JOB_ID,
      FIXTURE_REGISTRY_VERSION,
      FIXTURE_CANONICAL_TIMESTAMP
    );
    const { deriveCu, noopCu, deriveIxCu } = await measureDeriveCu(
      provider,
      payer,
      programId,
      seed,
      NODE_COUNT,
      groupSize
    );
    const row = {
      label: `sigReq=${sigReq}`,
      nodeCount: NODE_COUNT,
      groupSize,
      path: groupSize > NODE_COUNT / 2 ? "complement" : "direct",
      deriveCu,
      deriveIxCu,
      noopCu,
    };
    rows.push(row);
    console.log(row);
  }

  console.log("\n=== EVM reference vectors (bitmap.rs unit test) ===\n");
  for (const { nodeCount, groupSize, seed } of EVM_REF_CASES) {
    const { deriveCu, noopCu, deriveIxCu } = await measureDeriveCu(
      provider,
      payer,
      programId,
      seed,
      nodeCount,
      groupSize
    );
    const row = {
      label: `evm n=${nodeCount} g=${groupSize}`,
      nodeCount,
      groupSize,
      path: groupSize > nodeCount / 2 ? "complement" : "direct",
      deriveCu,
      deriveIxCu,
      noopCu,
    };
    rows.push(row);
    console.log(row);
  }

  const prodRows = rows.filter((r) => r.label.startsWith("sigReq="));
  const summary = {
    note: "deriveCu = CU(bench_derive_group_bitmap ix) - CU(bench_noop ix); isolates derive_group_bitmap + Anchor dispatch delta.",
    noopBaselineCu: noopBaseline,
    nodeCount: NODE_COUNT,
    redundancyBuffer: REDUNDANCY_BUFFER,
    production: prodRows,
    evmReference: rows.filter((r) => r.label.startsWith("evm")),
    range: {
      deriveCuMin: Math.min(...rows.map((r) => r.deriveCu)),
      deriveCuMax: Math.max(...rows.map((r) => r.deriveCu)),
    },
  };

  console.log("\n=== SUMMARY ===");
  console.log(JSON.stringify(summary, null, 2));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
