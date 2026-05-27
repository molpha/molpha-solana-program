/* eslint-disable no-console */
/**
 * Shared helpers for data-update CU benches (bankrun + localnet).
 */
const anchor = require("@coral-xyz/anchor");
const { PublicKey } = require("@solana/web3.js");
const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const fs = require("fs");
const path = require("path");

const EVM_FIXTURE = require("../../tests/fixtures-json/verify-answer-evm.json");

const EVM_ROUND_ID_PREFIX = Buffer.from([
  0x05, 0xcc, 0x9a, 0xbf, 0xa8, 0x3c, 0xdc, 0x2d, 0xff, 0xa2, 0x8a, 0xe0, 0xd4, 0x68,
  0x59, 0x5e, 0xce, 0xd0, 0x56, 0x4a, 0x16, 0xf6, 0xed, 0x9a, 0x3e, 0x2c, 0x5b, 0x4e,
  0x65, 0x2d, 0x98, 0x26,
]);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");

const NODE_COUNT = 32;
const SIGNER_COUNTS = [3, 8, 18];
const ACCOUNT_LAMPORTS = 10_000_000;
const COMPUTE_UNIT_LIMIT = 500_000;

/** +10% margin over bankrun baseline (2026-05-27, SHA-256 selection derive). */
const CU_CEILINGS = {
  verify_data_update: { 3: 296_000, 8: 384_000, 18: 529_000 },
  submit_data_update: { 3: 300_000, 8: 387_000, 18: 532_000 },
};

const FIXTURE_JOB_ID = hexToBuffer(EVM_FIXTURE.dataUpdate.jobId);
const FIXTURE_CONFIG_HASH = hexToBuffer(EVM_FIXTURE.dataUpdate.configHash);
const FIXTURE_VALUE = hexToBuffer(EVM_FIXTURE.dataUpdate.value);
const FIXTURE_REGISTRY_VERSION = EVM_FIXTURE.dataUpdate.registryVersion;
const FIXTURE_CANONICAL_TIMESTAMP = EVM_FIXTURE.dataUpdate.canonicalTimestamp;
const FIXTURE_SIGNERS_BITMAP = uint256BE(BigInt(EVM_FIXTURE.schnorrSignature.signersBitmap));
const FIXTURE_SIG_S = hexToBuffer(EVM_FIXTURE.schnorrSignature.signature);
const FIXTURE_COMMITMENT = hexToBuffer(
  EVM_FIXTURE.schnorrSignature.commitment.toLowerCase()
);
const FIXTURE_NODE_PUBKEYS = EVM_FIXTURE.nodePubkeys.map((h) => hexToBuffer(h));

const ACCOUNTS_ROOT = path.join(__dirname, "..", ".cu-bench-localnet-accounts");

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

const MOLPHA_SELECTION_V1 = Buffer.from("MOLPHA_SELECTION_V1");
const SELECTION_DOMAIN = Buffer.from(keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE")));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;
const REDUNDANCY_BUFFER = 2;

function u256BeFromU64(counter) {
  const buf = Buffer.alloc(32);
  buf.writeBigUInt64BE(BigInt(counter), 24);
  return buf;
}

function selectionHashRound(seed, counter) {
  const preimage = Buffer.concat([seed, SELECTION_DOMAIN, u256BeFromU64(counter)]);
  return Buffer.from(keccak_256(preimage));
}

function fullMaskBigInt(nodeCount) {
  return nodeCount === 256 ? (1n << 256n) - 1n : (1n << BigInt(nodeCount)) - 1n;
}

function bigintToEvmBytes32(value) {
  return Buffer.from(value.toString(16).padStart(64, "0"), "hex");
}

function sampleWithoutReplacement(seed, nodeCount, groupSize) {
  const limit = BigInt(Math.floor(0xffffffff / nodeCount) * nodeCount);
  let bitmap = 0n;
  let selected = 0;
  let counter = 0;
  while (selected < groupSize) {
    if (counter >= DERIVE_GROUP_BITMAP_MAX_ROUNDS) {
      throw new Error("deriveGroupBitmap: max Keccak-256 rounds exceeded");
    }
    const digest = selectionHashRound(seed, counter);
    counter += 1;
    let word = bigintFromBytesBE(digest);
    for (let w = 0; w < 8 && selected < groupSize; w += 1) {
      const limb = Number(word >> 224n);
      word = (word << 32n) & ((1n << 256n) - 1n);
      if (BigInt(limb) < limit) {
        const pos = limb % nodeCount;
        const bit = 1n << BigInt(pos);
        if ((bitmap & bit) === 0n) {
          bitmap |= bit;
          selected += 1;
        }
      }
    }
  }
  return bitmap;
}

/** Port of `derive_group_bitmap` in `programs/molpha/src/utils/bitmap.rs`. */
function deriveGroupBitmap(seed, nodeCount, groupSize) {
  if (nodeCount === 0) {
    throw new Error("deriveGroupBitmap: nodeCount must be > 0");
  }
  if (nodeCount > 256) {
    throw new Error("deriveGroupBitmap: nodeCount > 256");
  }
  if (groupSize > nodeCount) {
    throw new Error("deriveGroupBitmap: groupSize > nodeCount");
  }
  if (groupSize === 0) {
    return Buffer.alloc(32);
  }
  if (groupSize === nodeCount) {
    return bigintToEvmBytes32(fullMaskBigInt(nodeCount));
  }

  const bitmap =
    groupSize > nodeCount / 2
      ? fullMaskBigInt(nodeCount) ^
        sampleWithoutReplacement(seed, nodeCount, nodeCount - groupSize)
      : sampleWithoutReplacement(seed, nodeCount, groupSize);
  return bigintToEvmBytes32(bitmap);
}

function deriveSelectionSeed(jobId, registryVersion, roundId = Buffer.alloc(32)) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        MOLPHA_SELECTION_V1,
        jobId,
        u32be(registryVersion),
        roundId,
      ])
    )
  );
}

function effectiveSelectionSize(signaturesRequired, redundancyBuffer, nodeCount) {
  return Math.min(nodeCount, signaturesRequired + redundancyBuffer);
}

function deriveSignersBitmapForJob(
  jobId,
  registryVersion,
  nodeCount,
  signaturesRequired,
  redundancyBuffer = REDUNDANCY_BUFFER,
  roundId = Buffer.alloc(32)
) {
  const selectionSize = effectiveSelectionSize(
    signaturesRequired,
    redundancyBuffer,
    nodeCount
  );
  const seed = deriveSelectionSeed(jobId, registryVersion, roundId);
  return deriveGroupBitmap(seed, nodeCount, selectionSize);
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
  configHash,
  signersBitmap,
  value,
  canonicalTimestamp,
}) {
  return keccakConcat([
    EVM_ROUND_ID_PREFIX,
    jobId,
    u32be(registryVersion),
    u32be(signaturesRequired),
    configHash,
    signersBitmap,
    value,
    u64be(canonicalTimestamp),
  ]);
}

function benchConfigHash(signerCount) {
  if (signerCount === 8) return FIXTURE_CONFIG_HASH;
  const b = Buffer.alloc(32, 0);
  Buffer.from(`molpha-cu-bench-${signerCount}`).copy(b);
  return b;
}

function deriveJobId(owner, apiConfigHash) {
  return Buffer.from(
    keccak_256(Buffer.concat([JOB_ID_PREFIX, owner.toBuffer(), apiConfigHash]))
  );
}

function loadProgramId() {
  const toml = require("toml");
  const anchorToml = toml.parse(
    fs.readFileSync(path.join(__dirname, "..", "..", "Anchor.toml"), "utf8")
  );
  return new PublicKey(anchorToml.programs.localnet.molpha);
}

function loadIdl() {
  const idlPath = path.join(__dirname, "..", "..", "target", "idl", "molpha.json");
  const raw = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  const programId = new PublicKey(raw.address);
  return { idl: raw, programId };
}

function findRegistryIndexPda(programId, index) {
  const indexBytes = Buffer.alloc(4);
  indexBytes.writeUInt32LE(index);
  return PublicKey.findProgramAddressSync([REGISTRY_INDEX_SEED, indexBytes], programId)[0];
}

function registryIndexRemainingAccounts(programId, signersBitmap) {
  const remaining = [];
  for (let bitPos = 0; bitPos < 256; bitPos++) {
    if (evmBitmapBitSet(signersBitmap, bitPos)) {
      remaining.push({
        pubkey: findRegistryIndexPda(programId, bitPos),
        isSigner: false,
        isWritable: false,
      });
    }
  }
  return remaining;
}

/** Deterministic keys indexed by registry slot (0..slotCount-1). */
function generateSignerMaterial(slotCount = NODE_COUNT) {
  const privKeys = [];
  const compressedPubkeys = [];
  for (let i = 0; i < slotCount; i++) {
    const skBytes = Buffer.from(
      keccak_256(
        Buffer.concat([
          Buffer.from("molpha-cu-bench-sk"),
          Buffer.from([slotCount]),
          Buffer.from([i]),
        ])
      )
    );
    privKeys.push(modN(bigintFromBytesBE(skBytes)));
    compressedPubkeys.push(Buffer.from(secp256k1.getPublicKey(skBytes, true)));
  }
  return { privKeys, compressedPubkeys };
}

function buildCoalitionSignature({
  signerCount,
  privKeys,
  compressedPubkeys,
  jobId,
  registryVersion,
  configHash,
  value,
  canonicalTimestamp,
  nodeCount = NODE_COUNT,
  redundancyBuffer = REDUNDANCY_BUFFER,
}) {
  const signersBitmap = deriveSignersBitmapForJob(
    jobId,
    registryVersion,
    nodeCount,
    signerCount,
    redundancyBuffer
  );
  const messageHash = computeMessageHash({
    jobId,
    registryVersion,
    signaturesRequired: signerCount,
    configHash,
    signersBitmap,
    value,
    canonicalTimestamp,
  });

  let coalitionPoint = secp256k1.Point.ZERO;
  let coalitionSk = 0n;
  for (let bit = 0; bit < 256; bit++) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    coalitionSk = modN(coalitionSk + privKeys[bit]);
    coalitionPoint = coalitionPoint.add(
      secp256k1.Point.fromHex(Buffer.from(compressedPubkeys[bit]).toString("hex"))
    );
  }
  const coalitionCompressed = Buffer.from(coalitionPoint.toRawBytes(true));
  const { s, commitmentAddr } = schnorrSignForVerifier({
    coalitionPubkeyCompressed: coalitionCompressed,
    messageHash,
    coalitionPrivkey: coalitionSk,
  });

  return { signersBitmap, s, commitmentAddr };
}

function buildSubmitArgs(signerCount, signatureMaterial, jobId) {
  const signersBitmap = signatureMaterial.signersBitmap;
  const s = signatureMaterial.s;
  const commitmentAddr = signatureMaterial.commitmentAddr;

  return {
    jobId: Array.from(jobId),
    registryVersion: FIXTURE_REGISTRY_VERSION,
    roundId: Array.from(Buffer.alloc(32)),
    signersBitmap: Array.from(signersBitmap),
    value: Array.from(FIXTURE_VALUE),
    canonicalTimestamp: new anchor.BN(FIXTURE_CANONICAL_TIMESTAMP),
    aggSigR: Array.from(Buffer.alloc(32, 0)),
    aggSigS: Array.from(s),
    commitmentAddr: Array.from(commitmentAddr),
  };
}

async function encodeAccount(program, accountName, fields) {
  const data = await program.coder.accounts.encode(accountName, fields);
  return Buffer.from(data);
}

function toValidatorAccountJson(pubkey, data, owner, lamports = ACCOUNT_LAMPORTS) {
  return JSON.stringify(
    {
      pubkey: pubkey.toBase58(),
      account: {
        lamports,
        data: [data.toString("base64"), "base64"],
        owner: owner.toBase58(),
        executable: false,
        rentEpoch: 18446744073709551615,
      },
    },
    null,
    2
  );
}

async function buildBenchAccountSet(program, programId, payer, signerCount, signerMaterial) {
  const configHash = benchConfigHash(signerCount);
  const jobId = deriveJobId(payer.publicKey, configHash);

  const [protocolConfigPda, protocolBump] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda, registryBump] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const [jobPda, jobBump] = PublicKey.findProgramAddressSync(
    [JOB_SEED, jobId],
    programId
  );
  const [feedPda, feedBump] = PublicKey.findProgramAddressSync(
    [FEED_SEED, jobId],
    programId
  );

  const accounts = [];

  const protocolData = await encodeAccount(program, "protocolConfig", {
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
  accounts.push({
    pubkey: protocolConfigPda,
    data: protocolData,
    owner: programId,
  });

  const registryData = await encodeAccount(program, "registryState", {
    authority: payer.publicKey,
    currentVersion: FIXTURE_REGISTRY_VERSION,
    previousVersion: 0,
    previousExpiresAt: new anchor.BN(0),
    currentNodeCount: NODE_COUNT,
    previousNodeCount: 0,
    lastTransitionType: { none: {} },
    transitionAccount: PublicKey.default,
    graceSecs: new anchor.BN(0),
    withdrawalCooldownSecs: new anchor.BN(0),
    maxNodeCount: NODE_COUNT,
    bump: registryBump,
  });
  accounts.push({
    pubkey: registryStatePda,
    data: registryData,
    owner: programId,
  });

  const jobData = await encodeAccount(program, "job", {
    jobId: Array.from(jobId),
    owner: payer.publicKey,
    delegates: Array(5).fill(PublicKey.default),
    delegateCount: 0,
    apiConfigHash: Array.from(configHash),
    decimals: 8,
    signaturesRequired: signerCount,
    createdAt: new anchor.BN(0),
    bump: jobBump,
  });
  accounts.push({ pubkey: jobPda, data: jobData, owner: programId });

  const feedData = await encodeAccount(program, "feed", {
    jobId: Array.from(jobId),
    registryVersion: 0,
    roundId: Array.from(Buffer.alloc(32)),
    canonicalTimestamp: new anchor.BN(0),
    value: Array.from(Buffer.alloc(32)),
    signersBitmap: Array.from(Buffer.alloc(32)),
    signaturesRequired: 0,
    lastUpdatedSlot: new anchor.BN(0),
    bump: feedBump,
  });
  accounts.push({ pubkey: feedPda, data: feedData, owner: programId });

  const pubkeysToSeed = signerMaterial.compressedPubkeys.slice(0, NODE_COUNT);

  for (let index = 0; index < pubkeysToSeed.length; index++) {
    const { x, y } = decompressSecp256k1(pubkeysToSeed[index]);
    const registryIndexPda = findRegistryIndexPda(programId, index);
    const bump = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, Buffer.from(new Uint8Array([index, 0, 0, 0]))],
      programId
    )[1];
    const indexData = await encodeAccount(program, "registryIndex", {
      index,
      node: PublicKey.unique(),
      secp256K1PubkeyX: Array.from(x),
      secp256K1PubkeyY: Array.from(y),
      bump,
    });
    accounts.push({
      pubkey: registryIndexPda,
      data: indexData,
      owner: programId,
    });
  }

  return {
    accounts,
    jobId,
    configHash,
    protocolConfigPda,
    registryStatePda,
    jobPda,
    feedPda,
  };
}

async function writeAccountDir(program, programId, payer, signerCount, signerMaterial) {
  const dir = path.join(ACCOUNTS_ROOT, `n${signerCount}`);
  fs.mkdirSync(dir, { recursive: true });

  const { accounts } = await buildBenchAccountSet(
    program,
    programId,
    payer,
    signerCount,
    signerMaterial
  );

  for (const { pubkey, data, owner } of accounts) {
    const file = path.join(dir, `${pubkey.toBase58()}.json`);
    fs.writeFileSync(
      file,
      toValidatorAccountJson(pubkey, data, owner)
    );
  }

  const payerFile = path.join(dir, `${payer.publicKey.toBase58()}.json`);
  fs.writeFileSync(
    payerFile,
    toValidatorAccountJson(
      payer.publicKey,
      Buffer.alloc(0),
      new PublicKey("11111111111111111111111111111111"),
      1_000_000_000_000
    )
  );

  return dir;
}

module.exports = {
  EVM_ROUND_ID_PREFIX,
  NODE_COUNT,
  REDUNDANCY_BUFFER,
  SIGNER_COUNTS,
  COMPUTE_UNIT_LIMIT,
  CU_CEILINGS,
  FIXTURE_JOB_ID,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  FIXTURE_CONFIG_HASH,
  ACCOUNTS_ROOT,
  hexToBuffer,
  evmBitmapBitSet,
  deriveSignersBitmapForJob,
  deriveSelectionSeed,
  effectiveSelectionSize,
  benchConfigHash,
  deriveJobId,
  loadProgramId,
  loadIdl,
  findRegistryIndexPda,
  registryIndexRemainingAccounts,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  buildBenchAccountSet,
  writeAccountDir,
};
