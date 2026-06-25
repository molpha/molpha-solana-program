/* eslint-disable no-console */
/**
 * Extended CU sweep + linear fit for verify_data_update.
 * Run: node scripts/cu-bench-data-update-breakdown.js
 */
const anchor = require("@coral-xyz/anchor");
const {
  PublicKey,
  Keypair,
  SystemProgram,
  Transaction,
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
  FIXTURE_CONFIG_HASH,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  registryIndexRemainingAccounts,
  deriveSignersBitmapForJob,
  evmBitmapBitSet,
  loadIdl,
} = require("./lib/cu-bench-data-update-common");

const REDUNDANCY_BUFFER = 2;
const SWEEP = [1, 2, 3, 5, 8, 10, 15, 18, 20, 25, 30, 32];
const ACCOUNT_LAMPORTS = 10_000_000;

function loadProgramId() {
  const anchorToml = toml.parse(
    fs.readFileSync(path.join(__dirname, "..", "Anchor.toml"), "utf8")
  );
  return new PublicKey(anchorToml.programs.localnet.molpha);
}

function remainingPopcount(signersBitmap) {
  let n = 0;
  for (let i = 0; i < 256; i++) if (evmBitmapBitSet(signersBitmap, i)) n++;
  return n;
}

function selectionSize(sigReq) {
  return Math.min(NODE_COUNT, sigReq + REDUNDANCY_BUFFER);
}

// --- bankrun helpers (from cu-bench-data-update-bankrun.js) ---
const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");

function decompressSecp256k1(compressed) {
  const point = secp256k1.Point.fromHex(
    Buffer.from(compressed).toString("hex")
  );
  const uncompressed = point.toRawBytes(false);
  return { x: uncompressed.subarray(1, 33), y: uncompressed.subarray(33, 65) };
}

async function setProgramAccount(
  context,
  program,
  address,
  accountName,
  fields
) {
  const data = await program.coder.accounts.encode(accountName, fields);
  context.setAccount(address, {
    lamports: ACCOUNT_LAMPORTS,
    owner: program.programId,
    executable: false,
    data: Buffer.from(data),
  });
}

async function seedRegistryIndices(context, program, compressedPubkeys) {
  for (let index = 0; index < compressedPubkeys.length; index++) {
    const { x, y } = decompressSecp256k1(compressedPubkeys[index]);
    const registryIndexPda = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, Buffer.from(new Uint8Array([index, 0, 0, 0]))],
      program.programId
    )[0];
    const bump = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, Buffer.from(new Uint8Array([index, 0, 0, 0]))],
      program.programId
    )[1];
    await setProgramAccount(
      context,
      program,
      registryIndexPda,
      "registryIndex",
      {
        index,
        node: PublicKey.unique(),
        secp256K1PubkeyX: Array.from(x),
        secp256K1PubkeyY: Array.from(y),
        bump,
      }
    );
  }
}

async function seedBenchAccounts(
  context,
  program,
  payer,
  sigReq,
  signerMaterial
) {
  const [protocolConfigPda, protocolBump] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    program.programId
  );
  const [registryStatePda, registryBump] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    program.programId
  );
  const [jobPda, jobBump] = PublicKey.findProgramAddressSync(
    [JOB_SEED, FIXTURE_JOB_ID],
    program.programId
  );
  const [feedPda, feedBump] = PublicKey.findProgramAddressSync(
    [FEED_SEED, FIXTURE_JOB_ID],
    program.programId
  );

  await setProgramAccount(
    context,
    program,
    protocolConfigPda,
    "protocolConfig",
    {
      authority: payer.publicKey,
      pendingAuthority: null,
      usdcMint: PublicKey.default,
      treasury: PublicKey.default,
      rewardPerSignature: new anchor.BN(0),
      gatewayFee: new anchor.BN(0),
      receiptSubmitterBonus: new anchor.BN(0),
      protocolFeeBps: 0,
      maxReceiptDelaySlots: new anchor.BN(0),
      registryCooldownSecs: new anchor.BN(0),
      previousGraceSecs: new anchor.BN(0),
      maxNodeCount: NODE_COUNT,
      redundancyBuffer: REDUNDANCY_BUFFER,
      minimumNodeDeposit: new anchor.BN(0),
      timelockExpiresAt: null,
      pendingChange: null,
      bump: protocolBump,
    }
  );

  await setProgramAccount(context, program, registryStatePda, "registryState", {
    authority: payer.publicKey,
    currentNodeCount: NODE_COUNT,
    graceSecs: new anchor.BN(0),
    redundancyBuffer: REDUNDANCY_BUFFER,
    currentVersion: FIXTURE_REGISTRY_VERSION,
    previousVersion: 0,
    previousExpiresAt: new anchor.BN(0),
    previousNodeCount: 0,
    lastTransitionType: { none: {} },
    removedOldIndex: 0,
    removedNode: PublicKey.default,
    movedOldIndex: 0xffffffff,
    movedNode: PublicKey.default,
    bump: registryBump,
  });

  await setProgramAccount(context, program, jobPda, "job", {
    jobId: Array.from(FIXTURE_JOB_ID),
    owner: payer.publicKey,
    apiConfigHash: Array.from(FIXTURE_CONFIG_HASH),
    decimals: 8,
    signaturesRequired: sigReq,
    createdAt: new anchor.BN(0),
    lastRequestTimestamp: new anchor.BN(0),
    bump: jobBump,
  });

  await setProgramAccount(context, program, feedPda, "feed", {
    jobId: Array.from(FIXTURE_JOB_ID),
    registryVersion: 0,
    canonicalTimestamp: new anchor.BN(0),
    value: Array.from(Buffer.alloc(32)),
    signersBitmap: Array.from(Buffer.alloc(32)),
    signaturesRequired: 0,
    lastUpdatedSlot: new anchor.BN(0),
    bump: feedBump,
  });

  await seedRegistryIndices(
    context,
    program,
    signerMaterial.compressedPubkeys.slice(0, NODE_COUNT)
  );

  return { protocolConfigPda, registryStatePda, jobPda, feedPda };
}

async function simulateIx(provider, payer, ix) {
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

async function measureVerify(provider, program, payer, sigReq) {
  const signerMaterial = generateSignerMaterial(NODE_COUNT);
  const signatureMaterial = buildCoalitionSignature({
    signerCount: sigReq,
    privKeys: signerMaterial.privKeys,
    compressedPubkeys: signerMaterial.compressedPubkeys,
    jobId: FIXTURE_JOB_ID,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    configHash: FIXTURE_CONFIG_HASH,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
  });

  const { protocolConfigPda, registryStatePda, jobPda } =
    await seedBenchAccounts(
      provider.context,
      program,
      payer,
      sigReq,
      signerMaterial
    );

  const args = buildSubmitArgs(sigReq, signatureMaterial, FIXTURE_JOB_ID);
  const remainingAccounts = registryIndexRemainingAccounts(
    program.programId,
    signatureMaterial.signersBitmap
  );

  const verifyIx = await program.methods
    .verifyDataUpdate(args)
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      job: jobPda,
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
    const bm = deriveSignersBitmapForJob(
      FIXTURE_JOB_ID,
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
    note: "Bench label sigReq = job.signatures_required; remaining_accounts = popcount(selection_bitmap) = min(32, sigReq+2) unless sigReq>=30.",
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
    },
  };

  console.log("\n=== SUMMARY ===");
  console.log(JSON.stringify(summary, null, 2));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
