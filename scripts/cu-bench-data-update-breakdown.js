/* eslint-disable no-console */
/**
 * Extended CU sweep + linear fit for verify_data_update.
 * Run: node scripts/cu-bench-data-update-breakdown.js
 */
const anchor = require("@coral-xyz/anchor");
const {
  Keypair,
  SystemProgram,
  Transaction,
  ComputeBudgetProgram,
} = require("@solana/web3.js");
const { BankrunProvider, startAnchor } = require("anchor-bankrun");
const fs = require("fs");
const path = require("path");

const {
  NODE_COUNT,
  COMPUTE_UNIT_LIMIT,
  FIXTURE_SOURCE_ID,
  FIXTURE_CONFIG_HASH,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  REDUNDANCY_BUFFER,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  nodeRemainingAccounts,
  buildBenchAccountSet,
  deriveSignersBitmap,
  evmBitmapBitSet,
  loadIdl,
  loadProgramId,
} = require("./lib/cu-bench-data-update-common");

const SWEEP = [1, 2, 3, 5, 8, 10, 15, 18, 20, 25, 30, 32];
const LAMPORTS = 10_000_000;

function remainingPopcount(signersBitmap) {
  let n = 0;
  for (let i = 0; i < 256; i++) if (evmBitmapBitSet(signersBitmap, i)) n++;
  return n;
}

function selectionSize(sigReq) {
  return Math.min(NODE_COUNT, sigReq + REDUNDANCY_BUFFER);
}

async function seedFromAccountSet(context, accountSet) {
  for (const { pubkey, data, owner } of accountSet.accounts) {
    context.setAccount(pubkey, {
      lamports: LAMPORTS,
      owner,
      executable: false,
      data: Buffer.from(data),
    });
  }
}

async function simulateIx(provider, payer, ix) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const tx = new Transaction()
    .add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT })
    )
    .add(ix);
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  tx.partialSign(payer);
  const sim = await provider.context.banksClient.simulateTransaction(tx);
  if (sim.result !== null) {
    throw new Error(
      `simulation failed: ${sim.result}\nlogs:\n${(
        sim.meta?.logMessages || []
      ).join("\n")}`
    );
  }
  return Number(sim.meta.computeUnitsConsumed);
}

async function measureVerify(provider, program, payer, sigReq) {
  const signerMaterial = generateSignerMaterial(NODE_COUNT);
  const signatureMaterial = buildCoalitionSignature({
    signerCount: sigReq,
    privKeys: signerMaterial.privKeys,
    compressedPubkeys: signerMaterial.compressedPubkeys,
    sourceId: FIXTURE_SOURCE_ID,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
  });

  const accountSet = await buildBenchAccountSet(
    program,
    program.programId,
    payer,
    sigReq,
    signerMaterial,
    {
      sourceId: FIXTURE_SOURCE_ID,
      registryVersion: FIXTURE_REGISTRY_VERSION,
    }
  );
  await seedFromAccountSet(provider.context, accountSet);

  const args = buildSubmitArgs(sigReq, signatureMaterial, FIXTURE_SOURCE_ID);
  const remainingAccounts = nodeRemainingAccounts(
    accountSet.nodePdas,
    signatureMaterial.signersBitmap
  );

  const verifyIx = await program.methods
    .verifyDataUpdate(args)
    .accountsPartial({
      registry: accountSet.registryPda,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  return simulateIx(provider, payer, verifyIx);
}

function linearFit(points) {
  const n = points.length;
  let sx = 0,
    sy = 0,
    sxx = 0,
    sxy = 0;
  for (const { x, y } of points) {
    sx += x;
    sy += y;
    sxx += x * x;
    sxy += x * y;
  }
  const slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
  const intercept = (sy - slope * sx) / n;
  return { slope, intercept };
}

async function main() {
  const soPath = path.join(__dirname, "..", "target", "deploy", "molpha.so");
  if (!fs.existsSync(soPath)) {
    throw new Error("missing target/deploy/molpha.so — run yarn build:molpha");
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
  provider.wallet = new anchor.Wallet(payer);
  anchor.setProvider(provider);
  const { idl } = loadIdl();
  const program = new anchor.Program(idl, provider);

  const rows = [];
  for (const sigReq of SWEEP) {
    const bm = deriveSignersBitmap(
      FIXTURE_SOURCE_ID,
      FIXTURE_REGISTRY_VERSION,
      NODE_COUNT,
      sigReq,
      REDUNDANCY_BUFFER,
      FIXTURE_CANONICAL_TIMESTAMP
    );
    const rem = remainingPopcount(bm);
    const sel = selectionSize(sigReq);
    const verifyCu = await measureVerify(provider, program, payer, sigReq);
    rows.push({ sigReq, selectionSize: sel, remainingAccounts: rem, verifyCu });
    console.log("row", rows[rows.length - 1]);
  }

  const fitRem = linearFit(
    rows.map((r) => ({ x: r.remainingAccounts, y: r.verifyCu }))
  );
  const fitSig = linearFit(rows.map((r) => ({ x: r.sigReq, y: r.verifyCu })));

  const summary = {
    note: "Bench label sigReq = signatures_required; remaining_accounts = popcount(selection_bitmap) = min(32, sigReq+2).",
    rows,
    linearFitByRemainingAccounts: {
      ...fitRem,
      formula: `verify_cu ≈ ${Math.round(fitRem.intercept)} + ${Math.round(
        fitRem.slope
      )} * remaining_accounts`,
    },
    linearFitBySigReq: {
      ...fitSig,
      formula: `verify_cu ≈ ${Math.round(fitSig.intercept)} + ${Math.round(
        fitSig.slope
      )} * sigReq`,
    },
    benchPoints: {
      sigReq3: rows.find((r) => r.sigReq === 3),
      sigReq8: rows.find((r) => r.sigReq === 8),
      sigReq18: rows.find((r) => r.sigReq === 18),
      sigReq32: rows.find((r) => r.sigReq === 32),
    },
  };

  console.log("\n=== SUMMARY ===");
  console.log(JSON.stringify(summary, null, 2));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
