import * as crypto from "crypto";
import { BN, Program } from "@coral-xyz/anchor";
import {
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import * as secp256k1 from "@noble/secp256k1";
import { Point } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
import evmFixture from "../fixtures-json/verify-answer-evm.json";
import { VerifyAnswerEvmFixture } from "./e2e-fixture-export";
import { TestContext } from "../setup";

const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1"))
);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
const NODE_SEED = Buffer.from("molpha_node");

/** Matches `VIRTUAL_INDEX` in `programs/molpha/src/state/node.rs`. */
export const VIRTUAL_INDEX = 0xffffffff;
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");
const SELECTION_SEED_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_V1"))
);
const SELECTION_DOMAIN = keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE"));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;

const ACCOUNT_LAMPORTS = 10_000_000;

export const EVM_FIXTURE_JOB_ID = hexToBytes(evmFixture.dataUpdate.jobId);
export const EVM_FIXTURE_CONFIG_HASH = hexToBytes(
  evmFixture.dataUpdate.configHash
);
export const EVM_FIXTURE_VALUE = hexToBytes(evmFixture.dataUpdate.value);
export const EVM_FIXTURE_NODE_PUBKEYS = evmFixture.nodePubkeys.map((h) =>
  hexToBytes(h)
);

export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  return Uint8Array.from(Buffer.from(clean, "hex"));
}

function uint256BE(n: number | bigint): Uint8Array {
  const buf = new Uint8Array(32);
  let v = BigInt(n);
  for (let i = 31; i >= 0 && v > 0n; i--) {
    buf[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return buf;
}

export function decompressSecp256k1(compressed: Uint8Array): {
  x: Uint8Array;
  y: Uint8Array;
} {
  const point = Point.fromHex(Buffer.from(compressed).toString("hex"));
  const uncompressed = point.toRawBytes(false);
  return {
    x: uncompressed.subarray(1, 33),
    y: uncompressed.subarray(33, 65),
  };
}

/** Secret keys for `verify-answer-evm.json` coalition (see `secretKeys` in fixture). */
function signerMaterialForVerifyAnswerEvm(): E2eSignerMaterial {
  return signerMaterialFromFixture(evmFixture as LoadedE2eFixture);
}

/** Freshly signed args for `verify-answer-evm.json` fields (no `configHash` in message hash). */
export function buildSignedVerifyAnswerEvmArgs(
  registryVersion: number = evmFixture.dataUpdate.registryVersion
) {
  return signE2eDataUpdate(
    registryVersion,
    signerMaterialForVerifyAnswerEvm(),
    evmUint256BitmapForFirstN(evmFixture.dataUpdate.signaturesRequired),
    EVM_FIXTURE_JOB_ID,
    {
      configHash: EVM_FIXTURE_CONFIG_HASH,
      value: EVM_FIXTURE_VALUE,
      canonicalTimestamp: evmFixture.dataUpdate.canonicalTimestamp,
    },
    evmFixture.dataUpdate.signaturesRequired
  ).submitArgs;
}

/** Static fixture args (registry version 1); signs at call time for on-chain e2e. */
export function buildEvmSubmitDataUpdateArgs() {
  return buildSignedVerifyAnswerEvmArgs(evmFixture.dataUpdate.registryVersion);
}

export type E2eSignerMaterial = {
  privKeys: bigint[];
  compressedPubkeys: Uint8Array[];
};

function modN(x: bigint): bigint {
  const r = x % SECP256K1_ORDER_N;
  return r >= 0n ? r : r + SECP256K1_ORDER_N;
}

function bigintFromBytesBE(bytes: Uint8Array): bigint {
  return BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
}

function bytes32FromBigintBE(x: bigint): Uint8Array {
  let v = x;
  if (v < 0n) {
    v = ((v % SECP256K1_ORDER_N) + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
  }
  v %= SECP256K1_ORDER_N;
  return Uint8Array.from(Buffer.from(v.toString(16).padStart(64, "0"), "hex"));
}

function keccakConcat(chunks: Uint8Array[]): Uint8Array {
  return keccak_256(Buffer.concat(chunks.map((c) => Buffer.from(c))));
}

function u32be(x: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setUint32(0, x >>> 0, false);
  return buf;
}

function u64be(x: number): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, BigInt(x), false);
  return buf;
}

export function evmUint256BitmapForFirstN(n: number): Uint8Array {
  const bm = new Uint8Array(32);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return bm;
}

function u256BeFromU64(counter: number): Uint8Array {
  const buf = new Uint8Array(32);
  new DataView(buf.buffer).setBigUint64(24, BigInt(counter), false);
  return buf;
}

function selectionHashRound(seed: Uint8Array, counter: number): Uint8Array {
  return keccak_256(
    Buffer.concat([
      Buffer.from(seed),
      Buffer.from(SELECTION_DOMAIN),
      Buffer.from(u256BeFromU64(counter)),
    ])
  );
}

function fullMaskBigInt(nodeCount: number): bigint {
  return nodeCount === 256 ? (1n << 256n) - 1n : (1n << BigInt(nodeCount)) - 1n;
}

function bigintToEvmBytes32(value: bigint): Uint8Array {
  const hex = value.toString(16).padStart(64, "0");
  return Uint8Array.from(Buffer.from(hex, "hex"));
}

function sampleWithoutReplacement(
  seed: Uint8Array,
  nodeCount: number,
  groupSize: number
): bigint {
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
export function deriveGroupBitmap(
  seed: Uint8Array,
  nodeCount: number,
  groupSize: number
): Uint8Array {
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
    return new Uint8Array(32);
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

export function deriveSelectionSeed(
  jobId: Uint8Array,
  registryVersion: number,
  canonicalTimestamp: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      SELECTION_SEED_PREFIX,
      Buffer.from(jobId),
      Buffer.from(u32be(registryVersion)),
      Buffer.from(u64be(canonicalTimestamp)),
    ])
  );
}

/** Mirrors `effective_selection_size` in `programs/molpha/src/utils/bitmap.rs`. */
export function effectiveSelectionSize(
  signaturesRequired: number,
  redundancyBuffer: number,
  nodeCount: number
): number {
  return Math.min(nodeCount, signaturesRequired + redundancyBuffer);
}

export function deriveSignersBitmapForJob(
  jobId: Uint8Array,
  registryVersion: number,
  nodeCount: number,
  signaturesRequired: number,
  redundancyBuffer = 2,
  canonicalTimestamp: number
): Uint8Array {
  const selectionSize = effectiveSelectionSize(
    signaturesRequired,
    redundancyBuffer,
    nodeCount
  );
  const seed = deriveSelectionSeed(jobId, registryVersion, canonicalTimestamp);
  return deriveGroupBitmap(seed, nodeCount, selectionSize);
}

export function deriveNodeId(
  owner: PublicKey,
  secp256k1PubkeyCompressed: Uint8Array,
  registrationSalt: Uint8Array
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      NODE_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(secp256k1PubkeyCompressed),
      Buffer.from(registrationSalt),
    ])
  );
}

/**
 * Deterministic secp256k1 keys for integration e2e (register + coalition sign).
 * Uses Node `crypto.createECDH('secp256k1')` so compressed points match libsecp256k1 on-chain.
 */
export function generateE2eSignerMaterial(signerCount = 8): E2eSignerMaterial {
  return generateE2eSignerMaterialFromSeed(
    "molpha-e2e-integration-sk",
    signerCount
  );
}

export function generateE2eSignerMaterialFromSeed(
  seed: string,
  signerCount = 8
): E2eSignerMaterial {
  const privKeys: bigint[] = [];
  const compressedPubkeys: Uint8Array[] = [];
  for (let i = 0; i < signerCount; i++) {
    const skSeed = keccak_256(
      Buffer.concat([
        Buffer.from(seed),
        Buffer.from([signerCount]),
        Buffer.from([i]),
      ])
    );
    const ecdh = crypto.createECDH("secp256k1");
    ecdh.setPrivateKey(Buffer.from(skSeed));
    const compressed = new Uint8Array(
      ecdh.getPublicKey(undefined, "compressed")
    );
    privKeys.push(modN(bigintFromBytesBE(ecdh.getPrivateKey())));
    compressedPubkeys.push(compressed);
  }
  return { privKeys, compressedPubkeys };
}

export type E2eDataUpdateFields = {
  configHash: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
};

export function bytesToHex0x(bytes: Uint8Array): string {
  return `0x${Buffer.from(bytes).toString("hex")}`;
}

/** EVM `uint256` bitmap integer when bits `0..signerCount-1` are set. */
export function evmSignersBitmapNumber(signerCount: number): number {
  let n = 0;
  for (let i = 0; i < signerCount; i++) {
    n |= 1 << i;
  }
  return n;
}

/** EVM `uint256` bitmap integer from a 32-byte big-endian bitmap. */
export function evmSignersBitmapNumberFromBytes(
  signersBitmap: Uint8Array
): number {
  const n = bigintFromBytesBE(signersBitmap);
  if (n > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error("signersBitmap exceeds JS safe integer");
  }
  return Number(n);
}

export type LoadedE2eFixture = VerifyAnswerEvmFixture & {
  secretKeys?: string[];
};

function parseFixturePrivkeyScalar(raw: string): bigint {
  if (raw.startsWith("0x")) {
    return modN(bigintFromBytesBE(hexToBytes(raw)));
  }
  return modN(BigInt(raw));
}

export function fixturePrivkeys(fixture: LoadedE2eFixture): bigint[] {
  const raw = fixture.secretKeys ?? fixture.nodePrivkeys;
  if (!raw?.length) {
    throw new Error("fixture missing secretKeys or nodePrivkeys");
  }
  return raw.map(parseFixturePrivkeyScalar);
}

export function signerMaterialFromFixture(
  fixture: LoadedE2eFixture
): E2eSignerMaterial {
  const signerCount = fixture.dataUpdate.signaturesRequired;
  const privKeys = fixturePrivkeys(fixture);
  if (privKeys.length !== signerCount) {
    throw new Error(
      `expected ${signerCount} secret keys, got ${privKeys.length}`
    );
  }
  const compressedPubkeys = fixture.nodePubkeys
    .slice(0, signerCount)
    .map((h) => hexToBytes(h));
  return { privKeys, compressedPubkeys };
}

function bitmapToNodeIndexesOneBased(bitmap: bigint): number[] {
  const indexes: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (((bitmap >> BigInt(bit)) & BigInt(1)) === BigInt(1)) {
      indexes.push(bit + 1);
    }
  }
  return indexes;
}

/** Fail fast if fixture fields are inconsistent before hitting on-chain paths. */
export function assertE2eFixtureIntegrity(fixture: LoadedE2eFixture): void {
  const signerCount = fixture.dataUpdate.signaturesRequired;
  const configHash = hexToBytes(fixture.configHash);
  const dataConfigHash = hexToBytes(fixture.dataUpdate.configHash);

  if (!Buffer.from(configHash).equals(Buffer.from(dataConfigHash))) {
    throw new Error("configHash != dataUpdate.configHash");
  }
  if (fixture.registryVersion !== fixture.dataUpdate.registryVersion) {
    throw new Error("registryVersion != dataUpdate.registryVersion");
  }
  if (fixture.nodePubkeys.length !== fixture.registeredNodeCount) {
    throw new Error(
      `nodePubkeys length ${fixture.nodePubkeys.length} != registeredNodeCount ${fixture.registeredNodeCount}`
    );
  }

  const bitmapInt = BigInt(fixture.schnorrSignature.signersBitmap);
  const expectedIndexes = bitmapToNodeIndexesOneBased(bitmapInt);
  if (
    JSON.stringify(expectedIndexes) !==
    JSON.stringify(fixture.nodeIndexesOneBased)
  ) {
    throw new Error("nodeIndexesOneBased does not match signersBitmap");
  }

  let popcount = 0;
  let v = bitmapInt;
  while (v > 0n) {
    popcount += Number(v & 1n);
    v >>= 1n;
  }
  if (popcount !== signerCount) {
    throw new Error(
      `signersBitmap popcount ${popcount} != signaturesRequired ${signerCount}`
    );
  }

  const material = signerMaterialFromFixture(fixture);
  for (let i = 0; i < signerCount; i++) {
    const derived = Point.BASE.multiply(material.privKeys[i]).toRawBytes(true);
    if (
      !Buffer.from(derived).equals(Buffer.from(material.compressedPubkeys[i]))
    ) {
      throw new Error(`secret key ${i} does not match nodePubkeys[${i}]`);
    }
  }

  for (const pk of fixture.nodePubkeys) {
    const bytes = hexToBytes(pk);
    if (bytes.length !== 33 || (bytes[0] !== 0x02 && bytes[0] !== 0x03)) {
      throw new Error(`invalid compressed secp256k1 pubkey: ${pk}`);
    }
  }

  const sigS = hexToBytes(fixture.schnorrSignature.signature);
  const commitment = hexToBytes(
    fixture.schnorrSignature.commitment.toLowerCase()
  );
  if (sigS.length !== 32 || sigS.every((b) => b === 0)) {
    throw new Error("invalid schnorr signature scalar");
  }
  if (commitment.length !== 20) {
    throw new Error("commitment must be 20 bytes");
  }
}

const SECP256K1_ORDER_BYTES = Buffer.from(
  "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
  "hex"
);

function isZero32(bytes: Uint8Array): boolean {
  return bytes.every((b) => b === 0);
}

function subBE256(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(32);
  let borrow = 0;
  for (let i = 31; i >= 0; i--) {
    let v = a[i] - b[i] - borrow;
    if (v < 0) {
      v += 256;
      borrow = 1;
    } else {
      borrow = 0;
    }
    out[i] = v;
  }
  return out;
}

function mulMod256(a: Uint8Array, b: Uint8Array): Uint8Array {
  const mod = SECP256K1_ORDER_N;
  const av = bigintFromBytesBE(a) % mod;
  const bv = bigintFromBytesBE(b) % mod;
  return bytes32FromBigintBE((av * bv) % mod);
}

function modReduceOnce32(hash: Uint8Array): Uint8Array {
  return bytes32FromBigintBE(bigintFromBytesBE(hash));
}

/** Mirrors `evm_schnorr_ecdsa_inputs` in `programs/molpha/src/utils/schnorr.rs`. */
export function evmSchnorrEcdsaInputs(params: {
  coalitionPubkeyCompressed: Uint8Array;
  messageHash: Uint8Array;
  signature: Uint8Array;
  commitment: Uint8Array;
}): { recoveryId: number; ecdsaSig: Uint8Array; ecdsaHash: Uint8Array } {
  const pubkey = params.coalitionPubkeyCompressed;
  const parityPrefix = pubkey[0];
  if (parityPrefix !== 0x02 && parityPrefix !== 0x03) {
    throw new Error("invalid compressed pubkey prefix");
  }
  const recoveryId = parityPrefix & 1;
  const px = pubkey.subarray(1, 33);
  const pxModN = modReduceOnce32(px);
  if (isZero32(pxModN)) {
    throw new Error("px mod n is zero");
  }

  const challengeHash = keccakConcat([
    px,
    Uint8Array.from([recoveryId]),
    params.messageHash,
    params.commitment,
  ]);
  const challenge = modReduceOnce32(challengeHash);
  const sigRed = modReduceOnce32(params.signature);
  const msgMul = mulMod256(sigRed, pxModN);
  const eEcdsa = subBE256(SECP256K1_ORDER_BYTES, msgMul);
  const ecdsaSMul = mulMod256(challenge, pxModN);
  const sEcdsa = subBE256(SECP256K1_ORDER_BYTES, ecdsaSMul);
  if (isZero32(sEcdsa)) {
    throw new Error("ecdsa s is zero");
  }

  const ecdsaSig = new Uint8Array(64);
  ecdsaSig.set(pxModN, 0);
  ecdsaSig.set(sEcdsa, 32);
  return { recoveryId, ecdsaSig, ecdsaHash: eEcdsa };
}

export function coalitionCompressedFromFixture(
  fixture: LoadedE2eFixture
): Uint8Array {
  const signerCount = fixture.dataUpdate.signaturesRequired;
  let coalitionPoint = Point.ZERO;
  for (let i = 0; i < signerCount; i++) {
    coalitionPoint = coalitionPoint.add(
      Point.fromHex(
        Buffer.from(hexToBytes(fixture.nodePubkeys[i])).toString("hex")
      )
    );
  }
  return coalitionPoint.toRawBytes(true);
}

export function computeFixtureMessageHash(
  fixture: LoadedE2eFixture
): Uint8Array {
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  return computeMessageHash({
    jobId: hexToBytes(fixture.dataUpdate.jobId),
    registryVersion: fixture.dataUpdate.registryVersion,
    signaturesRequired: fixture.dataUpdate.signaturesRequired,
    signersBitmap,
    value: hexToBytes(fixture.dataUpdate.value),
    canonicalTimestamp: fixture.dataUpdate.canonicalTimestamp,
  });
}

/** Off-chain Schnorr-recovery check (same math as on-chain `verify_data_update`). */
export function verifyFixtureSchnorrSignature(fixture: LoadedE2eFixture): void {
  const material = fixture.secretKeys?.length
    ? signerMaterialFromFixture(fixture)
    : signerMaterialForVerifyAnswerEvm();
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  const signed = signE2eDataUpdate(
    fixture.dataUpdate.registryVersion,
    material,
    signersBitmap,
    hexToBytes(fixture.dataUpdate.jobId),
    {
      configHash: hexToBytes(fixture.dataUpdate.configHash),
      value: hexToBytes(fixture.dataUpdate.value),
      canonicalTimestamp: fixture.dataUpdate.canonicalTimestamp,
    },
    fixture.dataUpdate.signaturesRequired
  );
  const coalition = coalitionCompressedFromFixture(fixture);
  const messageHash = computeFixtureMessageHash(fixture);
  const { recoveryId, ecdsaSig, ecdsaHash } = evmSchnorrEcdsaInputs({
    coalitionPubkeyCompressed: coalition,
    messageHash,
    signature: signed.aggSigS,
    commitment: signed.commitmentAddr,
  });

  const { ethers } = require("ethers") as typeof import("ethers");
  const recovered = ethers.recoverAddress(
    `0x${Buffer.from(ecdsaHash).toString("hex")}`,
    {
      r: `0x${Buffer.from(ecdsaSig.subarray(0, 32)).toString("hex")}`,
      s: `0x${Buffer.from(ecdsaSig.subarray(32)).toString("hex")}`,
      v: recoveryId + 27,
    }
  );
  if (
    recovered.toLowerCase() !==
    `0x${Buffer.from(signed.commitmentAddr).toString("hex")}`
  ) {
    throw new Error("Schnorr recovery address mismatch");
  }
}

/** Anchor `submitDataUpdate` / `verifyDataUpdate` args from an e2e JSON fixture. */
export function buildSubmitArgsFromE2eFixture(fixture: LoadedE2eFixture) {
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  const material = fixture.secretKeys?.length
    ? signerMaterialFromFixture(fixture)
    : signerMaterialForVerifyAnswerEvm();
  return signE2eDataUpdate(
    fixture.dataUpdate.registryVersion,
    material,
    signersBitmap,
    hexToBytes(fixture.dataUpdate.jobId),
    {
      configHash: hexToBytes(fixture.dataUpdate.configHash),
      value: hexToBytes(fixture.dataUpdate.value),
      canonicalTimestamp: fixture.dataUpdate.canonicalTimestamp,
    },
    fixture.dataUpdate.signaturesRequired
  ).submitArgs;
}

function computeMessageHash(params: {
  jobId: Uint8Array;
  registryVersion: number;
  signaturesRequired: number;
  signersBitmap: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
}): Uint8Array {
  return keccakConcat([
    MESSAGE_PREFIX,
    params.jobId,
    u32be(params.registryVersion),
    u32be(params.signaturesRequired),
    params.signersBitmap,
    params.value,
    u64be(params.canonicalTimestamp),
  ]);
}

function commitmentAddrFromPoint(r: Point): Uint8Array {
  const uncompressed = r.toRawBytes(false);
  const digest = keccak_256(uncompressed.subarray(1));
  return digest.subarray(12, 32);
}

function schnorrSignForVerifier(params: {
  coalitionPubkeyCompressed: Uint8Array;
  messageHash: Uint8Array;
  coalitionPrivkey: bigint;
}): { s: Uint8Array; commitmentAddr: Uint8Array } {
  const px = params.coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = params.coalitionPubkeyCompressed[0] & 1;

  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const r = Point.BASE.multiply(k);
  const commitmentAddr = commitmentAddrFromPoint(r);

  const challengeHash = keccakConcat([
    px,
    Uint8Array.from([recoveryId]),
    params.messageHash,
    commitmentAddr,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + e * modN(params.coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitmentAddr };
}

export type SignedE2eDataUpdate = {
  submitArgs: ReturnType<typeof buildEvmSubmitDataUpdateArgs>;
  aggSigS: Uint8Array;
  commitmentAddr: Uint8Array;
  signersBitmap: Uint8Array;
};

/** Build submit args + signature material for the live registry coalition. */
export function signE2eDataUpdate(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCountOrBitmap: number | Uint8Array,
  jobId: Uint8Array,
  dataUpdate: E2eDataUpdateFields,
  signaturesRequired?: number,
  nodeCount?: number,
  redundancyBuffer = 2
): SignedE2eDataUpdate {
  const selectionTimestamp = dataUpdate.canonicalTimestamp;
  const signersBitmap =
    typeof signerCountOrBitmap === "number"
      ? deriveSignersBitmapForJob(
          jobId,
          registryVersion,
          nodeCount ?? signerCountOrBitmap,
          signaturesRequired ?? signerCountOrBitmap,
          redundancyBuffer,
          selectionTimestamp
        )
      : signerCountOrBitmap;
  const signerCount = bitmapPopcountEvm(signersBitmap);
  const required = signaturesRequired ?? signerCount;
  const messageHash = computeMessageHash({
    jobId,
    registryVersion,
    signaturesRequired: required,
    signersBitmap,
    value: dataUpdate.value,
    canonicalTimestamp: dataUpdate.canonicalTimestamp,
  });

  let coalitionPoint = Point.ZERO;
  let coalitionSk = 0n;
  for (let bit = 0; bit < 256; bit++) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    if (bit >= signerMaterial.privKeys.length) {
      throw new Error(`missing signer material for registry index ${bit}`);
    }
    coalitionSk = modN(coalitionSk + signerMaterial.privKeys[bit]);
    coalitionPoint = coalitionPoint.add(
      Point.fromHex(
        Buffer.from(signerMaterial.compressedPubkeys[bit]).toString("hex")
      )
    );
  }
  const coalitionCompressed = coalitionPoint.toRawBytes(true);
  const { s, commitmentAddr } = schnorrSignForVerifier({
    coalitionPubkeyCompressed: coalitionCompressed,
    messageHash,
    coalitionPrivkey: coalitionSk,
  });

  const submitArgs = {
    jobId: Array.from(jobId),
    registryVersion,
    signaturesRequired: required,
    signersBitmap: Array.from(signersBitmap),
    value: Array.from(dataUpdate.value),
    canonicalTimestamp: new BN(dataUpdate.canonicalTimestamp),
    aggSigR: Array.from(new Uint8Array(32)),
    aggSigS: Array.from(s),
    commitmentAddr: Array.from(commitmentAddr),
  };

  return { submitArgs, aggSigS: s, commitmentAddr, signersBitmap };
}

/** On-chain `verify_data_update` return buffer length (`verify_data_update.rs`). */
export const VERIFY_DATA_UPDATE_RETURN_LEN = 72;

/** Pack expected return data: `value[32] | timestamp_be[8] | round_id[32]`. */
export function expectedVerifyDataUpdateReturnBytes(
  value: Uint8Array,
  canonicalTimestamp: number
): Uint8Array {
  const out = new Uint8Array(VERIFY_DATA_UPDATE_RETURN_LEN);
  out.set(value.subarray(0, 32), 0);
  out.set(u64be(canonicalTimestamp), 32);
  return out;
}

/**
 * Run `verify_data_update` via bankrun `tryProcessTransaction` and return program return data.
 */
export async function executeVerifyDataUpdateReturnData(
  ctx: TestContext,
  program: Program,
  signed: SignedE2eDataUpdate,
  accounts: {
    registryStatePda: PublicKey;
  },
  computeUnitLimit = 1_400_000
): Promise<Uint8Array> {
  const verifyIx = await program.methods
    .verifyDataUpdate(signed.submitArgs)
    .accountsPartial({
      registryState: accounts.registryStatePda,
    })
    .remainingAccounts(
      signerRemainingAccounts(
        program.programId,
        signed.submitArgs.signersBitmap
      )
    )
    .instruction();

  const [blockhash] =
    await ctx.provider.context.banksClient.getLatestBlockhash();
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnitLimit }),
    verifyIx
  );
  tx.feePayer = ctx.authority.publicKey;
  tx.recentBlockhash = blockhash;
  tx.sign(ctx.authority.payer);

  const result = await ctx.provider.context.banksClient.tryProcessTransaction(
    tx
  );
  if (result.result !== null) {
    const logs = result.meta?.logMessages?.join("\n") ?? "";
    throw new Error(`verify_data_update failed: ${result.result}\n${logs}`);
  }
  const returnData = result.meta?.returnData;
  if (!returnData) {
    throw new Error("verify_data_update did not set return data");
  }
  if (!returnData.programId.equals(program.programId)) {
    throw new Error(
      `return data program id mismatch: ${returnData.programId.toBase58()}`
    );
  }
  return new Uint8Array(returnData.data);
}

/** Build submit args signed for the live registry version and registered coalition. */
export function buildSignedEvmSubmitDataUpdateArgs(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCount = 8,
  jobId: Uint8Array = EVM_FIXTURE_JOB_ID,
  dataUpdate: E2eDataUpdateFields = {
    configHash: EVM_FIXTURE_CONFIG_HASH,
    value: EVM_FIXTURE_VALUE,
    canonicalTimestamp: evmFixture.dataUpdate.canonicalTimestamp,
  },
  nodeCount = signerCount,
  redundancyBuffer = 2
): ReturnType<typeof buildEvmSubmitDataUpdateArgs> {
  return signE2eDataUpdate(
    registryVersion,
    signerMaterial,
    signerCount,
    jobId,
    dataUpdate,
    signerCount,
    nodeCount,
    redundancyBuffer
  ).submitArgs;
}

const VALID_POP_SIG_S = Uint8Array.from(
  Buffer.concat([Buffer.alloc(31, 0), Buffer.from([1])])
);

/** Node PDA: `["molpha_node", node_owner]`. */
export function findNodePdaByOwner(
  programId: PublicKey,
  nodeOwner: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, nodeOwner.toBuffer()],
    programId
  )[0];
}

/** Fund a node-owner keypair and its USDC ATA in bankrun for `register_node`. */
export async function fundNodeOwnerInBankrun(
  ctx: TestContext,
  owner: Keypair,
  usdcMint: PublicKey,
  amount: bigint = BigInt(500_000_000_000)
): Promise<PublicKey> {
  const ownerAta = getAssociatedTokenAddressSync(usdcMint, owner.publicKey);

  ctx.provider.context.setAccount(owner.publicKey, {
    lamports: ACCOUNT_LAMPORTS,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  });

  const tokenData = Buffer.alloc(165);
  usdcMint.toBuffer().copy(tokenData, 0);
  owner.publicKey.toBuffer().copy(tokenData, 32);
  tokenData.writeBigUInt64LE(amount, 64);
  tokenData.writeUInt32LE(0, 72);
  tokenData.writeUInt8(1, 108);

  ctx.provider.context.setAccount(ownerAta, {
    lamports: 2_039_280,
    owner: TOKEN_PROGRAM_ID,
    executable: false,
    data: tokenData,
  });

  return ownerAta;
}

/** Args for `registerNode` (matches on-chain `RegisterNodeArgs`). */
export function buildRegisterNodeArgs(params: {
  newIndex: number;
  registrationSalt: Uint8Array;
  secp256k1PubkeyCompressed: Uint8Array;
  ip?: number[];
  port?: number;
  securityDeposit?: BN;
}) {
  const { x, y } = decompressSecp256k1(params.secp256k1PubkeyCompressed);
  return {
    newIndex: params.newIndex,
    registrationSalt: Array.from(params.registrationSalt),
    ip: params.ip ?? [127, 0, 0, 1],
    port: params.port ?? 8080 + params.newIndex,
    pubKeyX: Array.from(x),
    pubKeyY: Array.from(y),
    popSigR: Array.from(Buffer.alloc(32, 0)),
    popSigS: Array.from(VALID_POP_SIG_S),
    securityDeposit: params.securityDeposit ?? new BN(100_000_000),
  };
}

/** Protocol-config-owned ATA used as the shared treasury. */
export function getProtocolTreasury(
  protocolConfigPda: PublicKey,
  usdcMint: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, protocolConfigPda, true);
}

export async function registerNodesForE2e(
  ctx: TestContext,
  program: Program,
  accounts: {
    protocolConfigPda: PublicKey;
    registryStatePda: PublicKey;
    usdcMint: PublicKey;
    ownerUsdc: PublicKey;
    treasury: PublicKey;
  },
  compressedPubkeys: Uint8Array[],
  advanceClock: (ctx: TestContext, seconds: number) => Promise<void>,
  opts?: { startIndex?: number; finalize?: boolean }
): Promise<void> {
  const graceAdvanceSecs = 2;
  const startIndex = opts?.startIndex ?? 0;
  const shouldFinalize = opts?.finalize ?? true;

  for (let i = 0; i < compressedPubkeys.length; i++) {
    const index = startIndex + i;
    if (i > 0 || startIndex > 0) {
      await advanceClock(ctx, graceAdvanceSecs);
    }

    const secpCompressed = compressedPubkeys[i];
    if (secpCompressed.length !== 33) {
      throw new Error(
        `registerNodesForE2e: expected 33-byte secp pubkey at index ${index}, got ${secpCompressed.length}`
      );
    }

    const nodeOwnerKp = ctx.nodes[index];
    const nodeOwner = nodeOwnerKp.publicKey;
    const ownerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      nodeOwnerKp,
      accounts.usdcMint
    );

    const registrationSalt = Uint8Array.from(Buffer.alloc(32, index + 1));
    const nodeId = deriveNodeId(nodeOwner, secpCompressed, registrationSalt);
    const nodePda = findNodePdaByOwner(program.programId, nodeOwner);
    const indexBytes = Buffer.alloc(4);
    indexBytes.writeUInt32LE(index);
    const [registryIndexPda] = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, indexBytes],
      program.programId
    );

    const registerNodeArgs = buildRegisterNodeArgs({
      newIndex: index,
      registrationSalt,
      secp256k1PubkeyCompressed: secpCompressed,
    });

    await program.methods
      .registerNode(registerNodeArgs, Array.from(nodeId))
      .accountsPartial({
        adminAuthority: ctx.authority.publicKey,
        nodeOwner,
        protocolConfig: accounts.protocolConfigPda,
        registryState: accounts.registryStatePda,
        node: nodePda,
        registryIndex: registryIndexPda,
        usdcMint: accounts.usdcMint,
        ownerUsdc,
        treasury: accounts.treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([nodeOwnerKp])
      .rpc();
  }

  if (shouldFinalize) {
    await advanceClock(ctx, graceAdvanceSecs);
    await program.methods
      .finalizeRegistryTransition()
      .accountsPartial({
        registryState: accounts.registryStatePda,
        rentReceiver: ctx.authority.publicKey,
        virtualRegistryIndex: null,
      })
      .rpc();
  }
}

export async function createJobViaInstruction(
  ctx: TestContext,
  program: Program,
  accounts: {
    protocolConfigPda: PublicKey;
    planPda: PublicKey;
    subscriptionPda: PublicKey;
  },
  apiConfigHash: Uint8Array,
  opts?: { decimals?: number; signaturesRequired?: number }
): Promise<{ jobPda: PublicKey; feedPda: PublicKey; jobId: Uint8Array }> {
  const decimals = opts?.decimals ?? 8;
  const signaturesRequired = opts?.signaturesRequired ?? 8;
  const jobId = deriveJobId(
    ctx.authority.publicKey,
    apiConfigHash,
    signaturesRequired
  );
  const jobPda = findJobPda(program.programId, jobId);
  const feedPda = findFeedPda(program.programId, jobId);

  await program.methods
    .createJob(
      {
        apiConfigHash: Array.from(apiConfigHash),
        signaturesRequired,
        decimals,
      },
      Array.from(jobId)
    )
    .accountsPartial({
      creator: ctx.authority.publicKey,
      jobOwner: ctx.authority.publicKey,
      protocolConfig: accounts.protocolConfigPda,
      plan: accounts.planPda,
      job: jobPda,
      subscription: accounts.subscriptionPda,
      feed: feedPda,
      delegate: null,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  return { jobPda, feedPda, jobId };
}

export async function createEvmFixtureJobViaInstruction(
  ctx: TestContext,
  program: Program,
  accounts: {
    protocolConfigPda: PublicKey;
    planPda: PublicKey;
    subscriptionPda: PublicKey;
  }
): Promise<{ jobPda: PublicKey; feedPda: PublicKey; jobId: Uint8Array }> {
  return createJobViaInstruction(
    ctx,
    program,
    accounts,
    EVM_FIXTURE_CONFIG_HASH
  );
}

export function deriveJobId(
  owner: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      JOB_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired]),
    ])
  );
}

/** On-chain job id for the EVM fixture config hash and a given owner. */
export function deriveEvmFixtureJobId(owner: PublicKey): Uint8Array {
  return deriveJobId(
    owner,
    EVM_FIXTURE_CONFIG_HASH,
    evmFixture.dataUpdate.signaturesRequired
  );
}

export function findJobPda(programId: PublicKey, jobId: Uint8Array): PublicKey {
  return PublicKey.findProgramAddressSync(
    [JOB_SEED, Buffer.from(jobId)],
    programId
  )[0];
}

export function findFeedPda(
  programId: PublicKey,
  jobId: Uint8Array
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [FEED_SEED, Buffer.from(jobId)],
    programId
  )[0];
}

/** @deprecated Prefer `findJobPda(programId, deriveEvmFixtureJobId(owner))`. */
export function findEvmJobPda(programId: PublicKey): PublicKey {
  return findJobPda(programId, EVM_FIXTURE_JOB_ID);
}

/** @deprecated Prefer `findFeedPda(programId, deriveEvmFixtureJobId(owner))`. */
export function findEvmFeedPda(programId: PublicKey): PublicKey {
  return findFeedPda(programId, EVM_FIXTURE_JOB_ID);
}

export function findRegistryIndexPda(
  programId: PublicKey,
  index: number
): PublicKey {
  const indexBytes = Buffer.alloc(4);
  indexBytes.writeUInt32LE(index);
  return PublicKey.findProgramAddressSync(
    [REGISTRY_INDEX_SEED, indexBytes],
    programId
  )[0];
}

export function evmBitmapBitSet(
  signersBitmap: number[] | Uint8Array,
  bit: number
): boolean {
  const byteIndex = 31 - Math.floor(bit / 8);
  const bitInByte = bit % 8;
  const byte = signersBitmap[byteIndex] ?? 0;
  return ((byte >> bitInByte) & 1) === 1;
}

export function bitmapPopcountEvm(signersBitmap: Uint8Array): number {
  let count = 0;
  for (let bit = 0; bit < 256; bit++) {
    if (evmBitmapBitSet(signersBitmap, bit)) {
      count += 1;
    }
  }
  return count;
}

export type RemoveTransitionInfo = {
  removedOldIndex: number;
  transitionType: "removeTail" | "removeSwap";
  movedOldIndex?: number;
};

/** Map a previous-version signer bit to the registry index PDA that holds its pubkey. */
export function expectedRegistryIndex(
  bit: number,
  transition?: RemoveTransitionInfo
): number {
  if (!transition) {
    return bit;
  }
  if (bit === transition.removedOldIndex) {
    return VIRTUAL_INDEX;
  }
  if (
    transition.transitionType === "removeSwap" &&
    transition.movedOldIndex !== undefined &&
    bit === transition.movedOldIndex
  ) {
    return transition.removedOldIndex;
  }
  return bit;
}

/** One `RegistryIndex` per set bit in `signers_bitmap`, ascending bit order. */
export function signerRemainingAccounts(
  programId: PublicKey,
  signersBitmap: number[] | Uint8Array,
  transition?: RemoveTransitionInfo
) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit++) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    const registryIndex = expectedRegistryIndex(bit, transition);
    remaining.push({
      pubkey: findRegistryIndexPda(programId, registryIndex),
      isSigner: false,
      isWritable: false,
    });
  }
  return remaining;
}

/** Current-version path: dense indices `0..signerCount-1` with all bits set. */
export function evmRegistryIndexRemainingAccounts(
  programId: PublicKey,
  signerCount = 8
) {
  const signersBitmap = new Array<number>(32).fill(0);
  signersBitmap[31] = (1 << signerCount) - 1;
  return signerRemainingAccounts(programId, signersBitmap);
}

async function setProgramAccount(
  ctx: TestContext,
  program: Program,
  address: PublicKey,
  accountName: string,
  fields: Record<string, unknown>
): Promise<void> {
  const data = await program.coder.accounts.encode(accountName, fields);
  ctx.provider.context.setAccount(address, {
    lamports: ACCOUNT_LAMPORTS,
    owner: program.programId,
    executable: false,
    data: Buffer.from(data),
  });
}

/** Registry layout matching `verify-answer-evm.json` (version 1, eight signers). */
export async function seedEvmRegistryState(
  ctx: TestContext,
  program: Program,
  registryStatePda: PublicKey,
  registry: {
    authority: PublicKey;
    currentVersion: number;
    previousVersion: number;
    previousExpiresAt: BN;
    currentNodeCount: number;
    previousNodeCount: number;
    lastTransitionType: Record<string, unknown>;
    removedOldIndex?: number;
    removedNode?: PublicKey;
    movedOldIndex?: number;
    movedNode?: PublicKey;
    graceSecs: BN;
    withdrawalCooldownSecs: BN;
    maxNodeCount: number;
    bump: number;
  },
  nodeCount = 8,
  registryVersion = evmFixture.dataUpdate.registryVersion
): Promise<void> {
  await setProgramAccount(ctx, program, registryStatePda, "registryState", {
    ...registry,
    currentVersion: registryVersion,
    previousExpiresAt: new BN(0),
    currentNodeCount: nodeCount,
    previousNodeCount: 0,
    lastTransitionType: { none: {} },
  });
}

export async function seedEvmRegistryIndices(
  ctx: TestContext,
  program: Program,
  signerCount = 8
): Promise<void> {
  for (let index = 0; index < signerCount; index++) {
    const compressed = EVM_FIXTURE_NODE_PUBKEYS[index];
    const { x, y } = decompressSecp256k1(compressed);
    const registryIndexPda = findRegistryIndexPda(program.programId, index);
    const bump = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, Buffer.from(new Uint8Array([index, 0, 0, 0]))],
      program.programId
    )[1];

    await setProgramAccount(ctx, program, registryIndexPda, "registryIndex", {
      index,
      node: PublicKey.unique(),
      secp256K1PubkeyX: Array.from(x),
      secp256K1PubkeyY: Array.from(y),
      bump,
    });
  }
}

export async function seedEvmJobAndFeed(
  ctx: TestContext,
  program: Program,
  owner: PublicKey
): Promise<{ jobPda: PublicKey; feedPda: PublicKey }> {
  const jobPda = findEvmJobPda(program.programId);
  const feedPda = findEvmFeedPda(program.programId);
  const jobBump = PublicKey.findProgramAddressSync(
    [JOB_SEED, Buffer.from(EVM_FIXTURE_JOB_ID)],
    program.programId
  )[1];
  const feedBump = PublicKey.findProgramAddressSync(
    [FEED_SEED, Buffer.from(EVM_FIXTURE_JOB_ID)],
    program.programId
  )[1];

  const signaturesRequired = evmFixture.dataUpdate.signaturesRequired;
  await setProgramAccount(ctx, program, jobPda, "job", {
    jobId: Array.from(EVM_FIXTURE_JOB_ID),
    owner,
    apiConfigHash: Array.from(EVM_FIXTURE_CONFIG_HASH),
    decimals: 8,
    signaturesRequired,
    createdAt: new BN(0),
    lastRequestTimestamp: new BN(0),
    bump: jobBump,
  });

  await setProgramAccount(ctx, program, feedPda, "feed", {
    jobId: Array.from(EVM_FIXTURE_JOB_ID),
    registryVersion: 0,
    canonicalTimestamp: new BN(0),
    value: Array.from(new Uint8Array(32)),
    signersBitmap: Array.from(new Uint8Array(32)),
    signaturesRequired: 0,
    lastUpdatedSlot: new BN(0),
    bump: feedBump,
  });

  return { jobPda, feedPda };
}

/** Seed registry + indices for previous-version verify after a RemoveSwap (8-node EVM layout). */
export async function seedEvmRemoveSwapGraceState(
  ctx: TestContext,
  program: Program,
  registryStatePda: PublicKey,
  opts: {
    removedOldIndex: number;
    movedOldIndex: number;
    previousVersion: number;
    currentVersion: number;
    previousExpiresAt: BN;
    removedNode?: PublicKey;
    movedNode?: PublicKey;
    authority: PublicKey;
    graceSecs: BN;
    withdrawalCooldownSecs: BN;
    maxNodeCount: number;
    registryBump: number;
  }
): Promise<void> {
  const {
    removedOldIndex,
    movedOldIndex,
    previousVersion,
    currentVersion,
    previousExpiresAt,
  } = opts;

  await setProgramAccount(ctx, program, registryStatePda, "registryState", {
    authority: opts.authority,
    currentVersion,
    previousVersion,
    previousExpiresAt,
    currentNodeCount: 7,
    previousNodeCount: 8,
    lastTransitionType: { removeSwap: {} },
    removedOldIndex,
    removedNode: opts.removedNode ?? PublicKey.unique(),
    movedOldIndex,
    movedNode: opts.movedNode ?? PublicKey.unique(),
    graceSecs: opts.graceSecs,
    withdrawalCooldownSecs: opts.withdrawalCooldownSecs,
    maxNodeCount: opts.maxNodeCount,
    bump: opts.registryBump,
  });

  await seedEvmRegistryIndices(ctx, program, 7);

  const movedCompressed = EVM_FIXTURE_NODE_PUBKEYS[movedOldIndex];
  const { x: movedX, y: movedY } = decompressSecp256k1(movedCompressed);
  const gapPda = findRegistryIndexPda(program.programId, removedOldIndex);
  const gapIndexBytes = Buffer.alloc(4);
  gapIndexBytes.writeUInt32LE(removedOldIndex);
  const gapBump = PublicKey.findProgramAddressSync(
    [REGISTRY_INDEX_SEED, gapIndexBytes],
    program.programId
  )[1];
  await setProgramAccount(ctx, program, gapPda, "registryIndex", {
    index: removedOldIndex,
    node: PublicKey.unique(),
    secp256K1PubkeyX: Array.from(movedX),
    secp256K1PubkeyY: Array.from(movedY),
    bump: gapBump,
  });

  const removedCompressed = EVM_FIXTURE_NODE_PUBKEYS[removedOldIndex];
  const { x: removedX, y: removedY } = decompressSecp256k1(removedCompressed);
  const virtualPda = findRegistryIndexPda(program.programId, VIRTUAL_INDEX);
  const virtualIndexBytes = Buffer.alloc(4);
  virtualIndexBytes.writeUInt32LE(VIRTUAL_INDEX);
  const virtualBump = PublicKey.findProgramAddressSync(
    [REGISTRY_INDEX_SEED, virtualIndexBytes],
    program.programId
  )[1];
  await setProgramAccount(ctx, program, virtualPda, "registryIndex", {
    index: VIRTUAL_INDEX,
    node: PublicKey.unique(),
    secp256K1PubkeyX: Array.from(removedX),
    secp256K1PubkeyY: Array.from(removedY),
    bump: virtualBump,
  });
}
