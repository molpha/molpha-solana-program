/* eslint-disable no-console */
/**
 * Bankrun / LiteSVM CU bench for `verify_data_update` and `submit_data_update`.
 *
 * Reports `unitsConsumed` for signer counts 3 / 8 / 18 via BankrunProvider.simulate
 * (LiteSVM-backed), without a local validator.
 *
 * Prereq: `yarn build:molpha` so `target/deploy/molpha.so` matches the on-chain derive logic.
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
const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const fs = require("fs");
const path = require("path");
const toml = require("toml");

const {
  NODE_COUNT,
  FIXTURE_JOB_ID,
  FIXTURE_CONFIG_HASH,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  registryIndexRemainingAccounts,
  findRegistryIndexPda,
} = require("./lib/cu-bench-data-update-common");

const MESSAGE_PREFIX = Buffer.from(keccak_256(Buffer.from("MOLPHA_MESSAGE_V1")));

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");

const SIGNER_COUNTS = [3, 8, 18];
const ACCOUNT_LAMPORTS = 10_000_000;
const COMPUTE_UNIT_LIMIT = 500_000;

/** +10% margin over bankrun baseline (2026-05-27, SHA-256 selection derive). */
const CU_CEILINGS = {
  verify_data_update: { 3: 296_000, 8: 384_000, 18: 529_000 },
  submit_data_update: { 3: 300_000, 8: 387_000, 18: 532_000 },
};


function hexToBuffer(hex) {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  return Buffer.from(clean, "hex");
}

function u32be(x) {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(x >>> 0, 0);
  return b;
}

function u64be(x) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64BE(BigInt(x), 0);
  return b;
}

function uint256BE(n) {
  const bm = Buffer.alloc(32, 0);
  let v = BigInt(n);
  for (let i = 31; i >= 0 && v > 0n; i--) {
    bm[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return bm;
}

function evmUint256BitmapForFirstN(n) {
  const bm = Buffer.alloc(32, 0);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return bm;
}

function evmBitmapBitSet(bitmap, pos) {
  const byte = 31 - (pos >> 3);
  const bit = pos & 7;
  return ((bitmap[byte] >> bit) & 1) === 1;
}

function bigintFromBytesBE(b) {
  return BigInt("0x" + Buffer.from(b).toString("hex"));
}

function modN(x) {
  const r = x % SECP256K1_ORDER_N;
  return r >= 0n ? r : r + SECP256K1_ORDER_N;
}

function bytes32FromBigintBE(x) {
  let v = x;
  if (v < 0n) v = (v % SECP256K1_ORDER_N + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
  v %= SECP256K1_ORDER_N;
  return Buffer.from(v.toString(16).padStart(64, "0"), "hex");
}

function keccakConcat(chunks) {
  return Buffer.from(keccak_256(Buffer.concat(chunks)));
}

function commitmentAddrFromPoint(R) {
  const uncompressed = Buffer.from(R.toRawBytes(false));
  const digest = Buffer.from(keccak_256(uncompressed.subarray(1)));
  return digest.subarray(12, 32);
}

function decompressSecp256k1(compressed) {
  const point = secp256k1.Point.fromHex(Buffer.from(compressed).toString("hex"));
  const uncompressed = point.toRawBytes(false);
  return {
    x: uncompressed.subarray(1, 33),
    y: uncompressed.subarray(33, 65),
  };
}

function schnorrSignForVerifier({ coalitionPubkeyCompressed, messageHash, coalitionPrivkey }) {
  const px = coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = coalitionPubkeyCompressed[0] & 1;

  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const R = secp256k1.Point.BASE.multiply(k);
  const commitmentAddr = commitmentAddrFromPoint(R);

  const challengeHash = keccakConcat([
    Buffer.from(px),
    Buffer.from([recoveryId]),
    messageHash,
    commitmentAddr,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + e * modN(coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitmentAddr };
}

function computeMessageHash({
  jobId,
  registryVersion,
  signaturesRequired,
  signersBitmap,
  value,
  canonicalTimestamp,
}) {
  return keccakConcat([
    MESSAGE_PREFIX,
    jobId,
    u32be(registryVersion),
    u32be(signaturesRequired),
    signersBitmap,
    value,
    u64be(canonicalTimestamp),
  ]);
}

function loadProgramId() {
  const anchorToml = toml.parse(fs.readFileSync(path.join(__dirname, "..", "Anchor.toml"), "utf8"));
  return new PublicKey(anchorToml.programs.localnet.molpha);
}

function loadIdl() {
  const idlPath = path.join(__dirname, "..", "target", "idl", "molpha.json");
  const raw = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  return { idl: raw, programId: new PublicKey(raw.address) };
}

async function setProgramAccount(context, program, address, accountName, fields) {
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
    const registryIndexPda = findRegistryIndexPda(program.programId, index);
    const bump = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, Buffer.from(new Uint8Array([index, 0, 0, 0]))],
      program.programId
    )[1];

    await setProgramAccount(context, program, registryIndexPda, "registryIndex", {
      index,
      node: PublicKey.unique(),
      secp256K1PubkeyX: Array.from(x),
      secp256K1PubkeyY: Array.from(y),
      bump,
    });
  }
}

async function seedBenchAccounts(context, program, payer, signerCount, signerMaterial) {
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

  await setProgramAccount(context, program, protocolConfigPda, "protocolConfig", {
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
    redundancyBuffer: 2,
    minimumNodeDeposit: new anchor.BN(0),
    timelockExpiresAt: null,
    pendingChange: null,
    bump: protocolBump,
  });

  await setProgramAccount(context, program, registryStatePda, "registryState", {
    authority: payer.publicKey,
    currentNodeCount: NODE_COUNT,
    graceSecs: new anchor.BN(0),
    redundancyBuffer: 2,
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
    delegates: Array(5).fill(PublicKey.default),
    delegateCount: 0,
    apiConfigHash: Array.from(FIXTURE_CONFIG_HASH),
    decimals: 8,
    signaturesRequired: signerCount,
    createdAt: new anchor.BN(0),
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

  await seedRegistryIndices(context, program, signerMaterial.compressedPubkeys.slice(0, NODE_COUNT));

  return { protocolConfigPda, registryStatePda, jobPda, feedPda };
}

async function simulateIx(provider, payer, ix, { skipComputeBudget = false } = {}) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const tx = new Transaction();
  if (!skipComputeBudget) {
    tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }));
  }
  tx.add(ix);
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  tx.partialSign(payer);

  let sim;
  try {
    sim = await provider.context.banksClient.simulateTransaction(tx);
  } catch (err) {
    if (!skipComputeBudget && String(err?.message || err).includes("Transaction too large")) {
      return simulateIx(provider, payer, ix, { skipComputeBudget: true });
    }
    throw err;
  }
  if (sim.result !== null) {
    throw new Error(
      `simulation failed: ${sim.result}\nlogs:\n${(sim.meta?.logMessages || []).join("\n")}`
    );
  }
  if (!sim.meta) {
    throw new Error("simulateTransaction returned no meta");
  }
  return Number(sim.meta.computeUnitsConsumed);
}

async function measureForSignerCount(provider, program, payer, signerCount) {
  const signerMaterial = generateSignerMaterial(NODE_COUNT);

  const signatureMaterial = buildCoalitionSignature({
    signerCount,
    privKeys: signerMaterial.privKeys,
    compressedPubkeys: signerMaterial.compressedPubkeys,
    jobId: FIXTURE_JOB_ID,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    configHash: FIXTURE_CONFIG_HASH,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
  });

  const { protocolConfigPda, registryStatePda, jobPda, feedPda } =
    await seedBenchAccounts(
      provider.context,
      program,
      payer,
      signerCount,
      signerMaterial
    );

  const args = buildSubmitArgs(signerCount, signatureMaterial, FIXTURE_JOB_ID);
  const signersBitmapBuf = signatureMaterial.signersBitmap;
  const remainingAccounts = registryIndexRemainingAccounts(
    program.programId,
    signersBitmapBuf
  );

  const verifyIx = await program.methods
    .verifyDataUpdate(args)
    .accountsPartial({
      registryState: registryStatePda,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const submitIx = await program.methods
    .submitDataUpdate(args)
    .accountsPartial({
      submitter: payer.publicKey,
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      feed: feedPda,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const verifyCu = await simulateIx(provider, payer, verifyIx);
  let submitCu;
  try {
    submitCu = await simulateIx(provider, payer, submitIx);
  } catch (err) {
    if (String(err?.message || err).includes("Transaction too large")) {
      console.warn(
        `submit_data_update tx exceeds legacy 1232-byte limit for signerCount=${signerCount}; skipping submit CU`
      );
      submitCu = null;
    } else {
      throw err;
    }
  }

  return { verifyCu, submitCu };
}

async function main() {
  const repoRoot = path.join(__dirname, "..");
  const soPath = path.join(repoRoot, "target", "deploy", "molpha.so");
  const bitmapSrc = path.join(repoRoot, "programs", "molpha", "src", "utils", "bitmap.rs");
  if (!fs.existsSync(soPath)) {
    throw new Error(
      "target/deploy/molpha.so missing — run `yarn build:molpha` first (anchor build alone does not refresh the .so)"
    );
  }
  if (fs.existsSync(bitmapSrc) && fs.statSync(bitmapSrc).mtimeMs > fs.statSync(soPath).mtimeMs) {
    throw new Error(
      "target/deploy/molpha.so is older than programs/molpha/src/utils/bitmap.rs — run `yarn build:molpha`"
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
          lamports: 1_000_000_000_000,
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

  const { idl, programId: idlProgramId } = loadIdl();
  const program = new anchor.Program(idl, provider);
  if (!program.programId.equals(idlProgramId)) {
    throw new Error("IDL program id mismatch");
  }

  const out = {
    runtime: "bankrun/litesvm",
    signerCounts: SIGNER_COUNTS,
    verify_data_update: {},
    submit_data_update: {},
  };

  for (const n of SIGNER_COUNTS) {
    const { verifyCu, submitCu } = await measureForSignerCount(
      provider,
      program,
      payer,
      n
    );
    out.verify_data_update[`cu${n}`] = verifyCu;
    out.submit_data_update[`cu${n}`] = submitCu;
    console.log("measured", {
      signerCount: n,
      verify_data_update: verifyCu,
      submit_data_update: submitCu,
    });
  }

  console.log("data_update CU (bankrun)", JSON.stringify(out, null, 2));

  const failures = [];
  for (const n of SIGNER_COUNTS) {
    const key = `cu${n}`;
    const verifyCu = out.verify_data_update[key];
    const submitCu = out.submit_data_update[key];
    const verifyCeil = CU_CEILINGS.verify_data_update[n];
    const submitCeil = CU_CEILINGS.submit_data_update[n];
    if (verifyCu > verifyCeil) {
      failures.push(
        `verify_data_update @ ${n} signers: ${verifyCu} CU > ceiling ${verifyCeil}`
      );
    }
    if (submitCu != null && submitCu > submitCeil) {
      failures.push(
        `submit_data_update @ ${n} signers: ${submitCu} CU > ceiling ${submitCeil}`
      );
    }
  }
  if (failures.length > 0) {
    console.error("CU ceiling check failed:\n" + failures.join("\n"));
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
