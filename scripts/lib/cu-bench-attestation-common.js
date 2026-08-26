/* eslint-disable no-console */
/**
 * Shared helpers for attestation CU benches (bankrun + localnet).
 *
 * Matches the current immutable versioned registry:
 * - `RegistryState` pointer PDA
 * - versioned zero-copy `Registry` snapshots (`["molpha_registry", version_le]`)
 * - permanent owner-keyed `Node` PDAs (`["molpha_node", owner]`)
 */
const anchor = require("@coral-xyz/anchor");
const { Keypair, PublicKey } = require("@solana/web3.js");
const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const EVM_FIXTURE = require("../../tests/fixtures-json/verify-answer-evm.json");

const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1"))
);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const REGISTRY_SEED = Buffer.from("molpha_registry");
const NODE_SEED = Buffer.from("molpha_node");
const FEED_SEED = Buffer.from("molpha_feed");
const FEED_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");

const NODE_COUNT = 32;
const SIGNER_COUNTS = [3, 8, 18, 32];
const ACCOUNT_LAMPORTS = 10_000_000;
const COMPUTE_UNIT_LIMIT = 1_400_000;
const REGISTRY_SPACE = 8_208;

/** Soft ceilings (~10% over bankrun baseline 2026-08-03, versioned Registry + Node PDAs). */
const CU_CEILINGS = {
  verify_attestation: { 3: 254_000, 8: 315_000, 18: 436_000, 32: 556_000 },
  submit_attestation: { 3: 273_000, 8: 334_000, 18: 454_000, 32: 584_000 },
};

const FIXTURE_SOURCE_ID = hexToBuffer(
  EVM_FIXTURE.attestation.sourceId ?? EVM_FIXTURE.attestation.feedId
);
/** @deprecated Use FIXTURE_SOURCE_ID. */
const FIXTURE_FEED_ID = FIXTURE_SOURCE_ID;
const FIXTURE_VALUE = hexToBuffer(EVM_FIXTURE.attestation.value);
const FIXTURE_REGISTRY_VERSION = EVM_FIXTURE.attestation.registryVersion;
const FIXTURE_CANONICAL_TIMESTAMP = EVM_FIXTURE.attestation.canonicalTimestamp;

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

function u8be(x) {
  return Buffer.from([x & 0xff]);
}

function u32le(x) {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(x >>> 0, 0);
  return b;
}

function u64be(x) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64BE(BigInt(x), 0);
  return b;
}

function evmBitmapBitSet(bitmap, pos) {
  const byte = 31 - (pos >> 3);
  const bit = pos & 7;
  return ((bitmap[byte] >> bit) & 1) === 1;
}

const SELECTION_SEED_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_V1"))
);
const SELECTION_DOMAIN = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE"))
);
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;
const REDUNDANCY_BUFFER = 2;

function u256BeFromU64(counter) {
  const buf = Buffer.alloc(32);
  buf.writeBigUInt64BE(BigInt(counter), 24);
  return buf;
}

function selectionHashRound(seed, counter) {
  const preimage = Buffer.concat([
    seed,
    SELECTION_DOMAIN,
    u256BeFromU64(counter),
  ]);
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

function deriveSelectionSeed(sourceId, registryVersion, canonicalTimestamp) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        SELECTION_SEED_PREFIX,
        sourceId,
        u32be(registryVersion),
        u64be(canonicalTimestamp),
      ])
    )
  );
}

function effectiveSelectionSize(
  signaturesRequired,
  redundancyBuffer,
  nodeCount
) {
  return Math.min(nodeCount, signaturesRequired + redundancyBuffer);
}

function deriveSignersBitmap(
  sourceId,
  registryVersion,
  nodeCount,
  signaturesRequired,
  redundancyBuffer = REDUNDANCY_BUFFER,
  timestamp
) {
  const selectionSize = effectiveSelectionSize(
    signaturesRequired,
    redundancyBuffer,
    nodeCount
  );
  const seed = deriveSelectionSeed(sourceId, registryVersion, timestamp);
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
  if (v < 0n)
    v = ((v % SECP256K1_ORDER_N) + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
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
  const point = secp256k1.Point.fromHex(
    Buffer.from(compressed).toString("hex")
  );
  const uncompressed = point.toRawBytes(false);
  return {
    x: uncompressed.subarray(1, 33),
    y: uncompressed.subarray(33, 65),
  };
}

function schnorrSignForVerifier({
  coalitionPubkeyCompressed,
  messageHash,
  coalitionPrivkey,
}) {
  const px = coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = coalitionPubkeyCompressed[0] & 1;

  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const R = secp256k1.Point.BASE.multiply(k);
  const commitment = commitmentAddrFromPoint(R);

  const challengeHash = keccakConcat([
    Buffer.from(px),
    Buffer.from([recoveryId]),
    messageHash,
    commitment,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + e * modN(coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitment };
}

/**
 * Must match `molpha_verifier::message::compute_message_hash`:
 * `MESSAGE_PREFIX || value || sourceId || registryVersion_u32be ||
 *  signaturesRequired_u8 || canonicalTimestamp_u64be || signersBitmap`.
 */
function computeMessageHash({
  sourceId,
  registryVersion,
  signaturesRequired,
  signersBitmap,
  value,
  canonicalTimestamp,
}) {
  return keccakConcat([
    MESSAGE_PREFIX,
    value,
    sourceId,
    u32be(registryVersion),
    u8be(signaturesRequired),
    u64be(canonicalTimestamp),
    signersBitmap,
  ]);
}

function benchConfigHash(signerCount) {
  const b = Buffer.alloc(32, 0);
  Buffer.from(`molpha-cu-bench-${signerCount}`).copy(b);
  return b;
}

function deriveFeedId(owner, apiConfigHash, signaturesRequired) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        FEED_ID_PREFIX,
        owner.toBuffer(),
        apiConfigHash,
        Buffer.from([signaturesRequired]),
      ])
    )
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
  const idlPath = path.join(
    __dirname,
    "..",
    "..",
    "target",
    "idl",
    "molpha.json"
  );
  const raw = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  const programId = new PublicKey(raw.address);
  return { idl: raw, programId };
}

function accountDiscriminator(name) {
  return crypto
    .createHash("sha256")
    .update(`account:${name}`)
    .digest()
    .subarray(0, 8);
}

function findRegistryPda(programId, version) {
  return PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, u32le(version)],
    programId
  )[0];
}

function findNodePda(programId, owner) {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, owner.toBuffer()],
    programId
  )[0];
}

function findFeedPda(programId, sourceId, signaturesRequired, submitter) {
  return PublicKey.findProgramAddressSync(
    [
      FEED_SEED,
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      submitter.toBuffer(),
    ],
    programId
  )[0];
}

/** Deterministic owner keypairs for bench Node PDAs (index 0..slotCount-1). */
function deterministicNodeOwner(index) {
  const seed = Buffer.from(
    keccak_256(
      Buffer.concat([
        Buffer.from("molpha-cu-bench-owner"),
        Buffer.from([index & 0xff]),
        Buffer.from([(index >> 8) & 0xff]),
      ])
    )
  );
  return Keypair.fromSeed(seed);
}

/** One Node PDA per set bit, ascending bit order (matches on-chain resolver). */
function nodeRemainingAccounts(nodePdas, signersBitmap) {
  const remaining = [];
  for (let bitPos = 0; bitPos < 256; bitPos++) {
    if (!evmBitmapBitSet(signersBitmap, bitPos)) continue;
    const pubkey = nodePdas[bitPos];
    if (!pubkey) {
      throw new Error(`Missing Node PDA for registry index ${bitPos}`);
    }
    remaining.push({
      pubkey,
      isSigner: false,
      isWritable: false,
    });
  }
  return remaining;
}

/** Deterministic secp keys + owner keypairs indexed by registry slot. */
function generateSignerMaterial(slotCount = NODE_COUNT) {
  const privKeys = [];
  const compressedPubkeys = [];
  const owners = [];
  const nodePdas = [];
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
    owners.push(deterministicNodeOwner(i));
  }
  return { privKeys, compressedPubkeys, owners, nodePdas };
}

function attachNodePdas(programId, signerMaterial) {
  const nodePdas = signerMaterial.owners.map((owner) =>
    findNodePda(programId, owner.publicKey)
  );
  return { ...signerMaterial, nodePdas };
}

function buildCoalitionSignature({
  signerCount,
  privKeys,
  compressedPubkeys,
  sourceId,
  registryVersion,
  value,
  canonicalTimestamp,
  nodeCount = NODE_COUNT,
  redundancyBuffer = REDUNDANCY_BUFFER,
}) {
  const signersBitmap = deriveSignersBitmap(
    sourceId,
    registryVersion,
    nodeCount,
    signerCount,
    redundancyBuffer,
    canonicalTimestamp
  );
  const messageHash = computeMessageHash({
    sourceId,
    registryVersion,
    signaturesRequired: signerCount,
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
      secp256k1.Point.fromHex(
        Buffer.from(compressedPubkeys[bit]).toString("hex")
      )
    );
  }
  const coalitionCompressed = Buffer.from(coalitionPoint.toRawBytes(true));
  const { s, commitment } = schnorrSignForVerifier({
    coalitionPubkeyCompressed: coalitionCompressed,
    messageHash,
    coalitionPrivkey: coalitionSk,
  });

  return { signersBitmap, s, commitment };
}

function buildSubmitArgs(signerCount, signatureMaterial, sourceId) {
  return buildLiveSubmitArgs({
    sourceId,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    signaturesRequired: signerCount,
    signersBitmap: signatureMaterial.signersBitmap,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
    aggSigS: signatureMaterial.s,
    commitment: signatureMaterial.commitment,
  });
}

/** Anchor `submitAttestation` / `verifyAttestation` args (current on-chain layout). */
function buildLiveSubmitArgs({
  sourceId,
  feedId,
  registryVersion,
  signaturesRequired,
  signersBitmap,
  value,
  canonicalTimestamp,
  aggSigS,
  commitment,
}) {
  const id = sourceId ?? feedId;
  return {
    sourceId: Array.from(id),
    signaturesRequired,
    registryVersion,
    signersBitmap: Array.from(signersBitmap),
    value: Buffer.from(value),
    canonicalTimestamp: new anchor.BN(canonicalTimestamp),
    aggSigS: Array.from(aggSigS),
    commitment: Array.from(commitment),
  };
}

function signerMaterialFromManifestNodes(nodes, nodeCount) {
  const privKeys = new Array(nodeCount).fill(0n);
  const compressedPubkeys = new Array(nodeCount).fill(null);
  const owners = new Array(nodeCount).fill(null);

  for (const node of nodes) {
    const idx = node.index;
    if (idx < 0 || idx >= nodeCount) {
      throw new Error(
        `Node index ${idx} is outside registry size ${nodeCount}; re-register or adjust --nodes`
      );
    }
    const skHex = (node.evm?.privateKey ?? "").replace(/^0x/, "");
    const pkHex =
      node.evm?.publicKeyCompressed0x ?? node.evm?.publicKeyCompressed ?? "";
    if (!skHex || !pkHex) {
      throw new Error(
        `Node ${idx} is missing EVM key material in the manifest`
      );
    }
    privKeys[idx] = modN(bigintFromBytesBE(Buffer.from(skHex, "hex")));
    compressedPubkeys[idx] = hexToBuffer(pkHex);
    if (node.owner) {
      owners[idx] =
        typeof node.owner === "string"
          ? { publicKey: new PublicKey(node.owner) }
          : node.owner;
    } else {
      owners[idx] = deterministicNodeOwner(idx);
    }
  }

  for (let i = 0; i < nodeCount; i++) {
    if (compressedPubkeys[i] === null) {
      throw new Error(
        `Manifest has no node for registry index ${i}; expected ${nodeCount} contiguous entries`
      );
    }
  }

  return { privKeys, compressedPubkeys, owners, nodeCount };
}

function signAttestationForRegistry({
  registryVersion,
  signerMaterial,
  signaturesRequired,
  sourceId,
  feedId,
  value,
  canonicalTimestamp,
  nodeCount,
  redundancyBuffer = REDUNDANCY_BUFFER,
}) {
  const id = sourceId ?? feedId;
  const signatureMaterial = buildCoalitionSignature({
    signerCount: signaturesRequired,
    privKeys: signerMaterial.privKeys,
    compressedPubkeys: signerMaterial.compressedPubkeys,
    sourceId: id,
    registryVersion,
    value,
    canonicalTimestamp,
    nodeCount,
    redundancyBuffer,
  });

  return {
    submitArgs: buildLiveSubmitArgs({
      sourceId: id,
      registryVersion,
      signaturesRequired,
      signersBitmap: signatureMaterial.signersBitmap,
      value,
      canonicalTimestamp,
      aggSigS: signatureMaterial.s,
      commitment: signatureMaterial.commitment,
    }),
    signersBitmap: signatureMaterial.signersBitmap,
  };
}

async function encodeAccount(program, accountName, fields) {
  const data = await program.coder.accounts.encode(accountName, fields);
  return Buffer.from(data);
}

/** Manual zero-copy layout for `Registry` / `BenchFullRegistry` (8,208 bytes). */
function encodeRegistrySnapshot({
  accountName = "Registry",
  version,
  members,
  bump,
  redundancyBuffer = REDUNDANCY_BUFFER,
}) {
  if (members.length > 256) throw new Error("registry exceeds 256 nodes");
  const data = Buffer.alloc(REGISTRY_SPACE);
  accountDiscriminator(accountName).copy(data, 0);
  data.writeUInt32LE(version >>> 0, 8);
  data.writeUInt16LE(members.length, 12);
  data.writeUInt8(redundancyBuffer, 14);
  data.writeUInt8(bump, 15);
  members.forEach((member, index) =>
    member.toBuffer().copy(data, 16 + index * 32)
  );
  return data;
}

function toValidatorAccountJson(
  pubkey,
  data,
  owner,
  lamports = ACCOUNT_LAMPORTS
) {
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

function protocolConfigFields(payer, protocolBump) {
  return {
    authority: payer.publicKey,
    usdcMint: PublicKey.default,
    treasury: PublicKey.default,
    rewardPerSignature: new anchor.BN(0),
    gatewayFee: new anchor.BN(0),
    receiptSubmitterBonus: new anchor.BN(0),
    protocolFeeBps: 0,
    maxReceiptDelaySlots: new anchor.BN(0),
    protocolFee: new anchor.BN(0),
    disputeWindowSlots: new anchor.BN(0),
    gatewayBondMin: new anchor.BN(0),
    challengerBountyBps: 0,
    maxNodeCount: NODE_COUNT,
    minimumNodeDeposit: new anchor.BN(0),
    withdrawalCooldownSecs: new anchor.BN(0),
    x402RoundBase: new anchor.BN(0),
    x402RoundPerSig: new anchor.BN(0),
    bump: protocolBump,
  };
}

async function buildBenchAccountSet(
  program,
  programId,
  payer,
  signerCount,
  signerMaterial,
  opts = {}
) {
  const sourceId = opts.sourceId;
  const registryVersion = opts.registryVersion ?? FIXTURE_REGISTRY_VERSION;
  const material = attachNodePdas(programId, signerMaterial);
  const nodeCount = material.compressedPubkeys.length;

  const [protocolConfigPda, protocolBump] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda, registryStateBump] =
    PublicKey.findProgramAddressSync([REGISTRY_STATE_SEED], programId);
  const [registryPda, registryBump] = PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, u32le(registryVersion)],
    programId
  );
  const feedPda = findFeedPda(
    programId,
    sourceId,
    signerCount,
    payer.publicKey
  );

  const accounts = [];

  const protocolData = await encodeAccount(
    program,
    "protocolConfig",
    protocolConfigFields(payer, protocolBump)
  );
  accounts.push({
    pubkey: protocolConfigPda,
    data: protocolData,
    owner: programId,
  });

  const registryStateData = await encodeAccount(program, "registryState", {
    currentVersion: registryVersion,
    bump: registryStateBump,
  });
  accounts.push({
    pubkey: registryStatePda,
    data: registryStateData,
    owner: programId,
  });

  const registryData = encodeRegistrySnapshot({
    version: registryVersion,
    members: material.nodePdas,
    bump: registryBump,
    redundancyBuffer: REDUNDANCY_BUFFER,
  });
  accounts.push({
    pubkey: registryPda,
    data: registryData,
    owner: programId,
  });

  // Do not pre-seed `feed`: `submit_attestation` uses `init_if_needed` with
  // `Feed::SPACE`. Anchor's account encoder pads `Vec` to `max_len`, which does
  // not match `InitSpace`, so a coder-encoded feed fails the space constraint.

  for (let index = 0; index < nodeCount; index++) {
    const owner = material.owners[index];
    const nodePda = material.nodePdas[index];
    const { x, y } = decompressSecp256k1(material.compressedPubkeys[index]);
    const bump = PublicKey.findProgramAddressSync(
      [NODE_SEED, owner.publicKey.toBuffer()],
      programId
    )[1];
    const nodeData = await encodeAccount(program, "node", {
      owner: owner.publicKey,
      secp256K1PubkeyX: Array.from(x),
      secp256K1PubkeyY: Array.from(y),
      status: { active: {} },
      ip: [127, 0, 0, 1],
      port: 9_000 + index,
      lockedAmount: new anchor.BN(0),
      claimableRewards: new anchor.BN(0),
      registeredAt: new anchor.BN(0),
      deactivatedAt: new anchor.BN(0),
      withdrawableAt: new anchor.BN(0),
      bump,
    });
    accounts.push({ pubkey: nodePda, data: nodeData, owner: programId });
  }

  return {
    accounts,
    sourceId,
    feedId: sourceId,
    registryVersion,
    protocolConfigPda,
    registryStatePda,
    registryPda,
    feedPda,
    nodePdas: material.nodePdas,
    signerMaterial: material,
  };
}

async function writeAccountDir(
  program,
  programId,
  payer,
  signerCount,
  signerMaterial
) {
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
    fs.writeFileSync(file, toValidatorAccountJson(pubkey, data, owner));
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
  MESSAGE_PREFIX,
  NODE_COUNT,
  REDUNDANCY_BUFFER,
  SIGNER_COUNTS,
  COMPUTE_UNIT_LIMIT,
  CU_CEILINGS,
  REGISTRY_SPACE,
  FIXTURE_SOURCE_ID,
  FIXTURE_FEED_ID,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  ACCOUNTS_ROOT,
  PROTOCOL_CONFIG_SEED,
  REGISTRY_STATE_SEED,
  REGISTRY_SEED,
  NODE_SEED,
  FEED_SEED,
  hexToBuffer,
  u32le,
  evmBitmapBitSet,
  deriveSignersBitmap,
  deriveSelectionSeed,
  effectiveSelectionSize,
  benchConfigHash,
  deriveFeedId,
  loadProgramId,
  loadIdl,
  findRegistryPda,
  findNodePda,
  findFeedPda,
  deterministicNodeOwner,
  nodeRemainingAccounts,
  generateSignerMaterial,
  attachNodePdas,
  buildCoalitionSignature,
  buildSubmitArgs,
  buildLiveSubmitArgs,
  signerMaterialFromManifestNodes,
  signAttestationForRegistry,
  encodeAccount,
  encodeRegistrySnapshot,
  protocolConfigFields,
  buildBenchAccountSet,
  writeAccountDir,
};
