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
import { signNodePop } from "../../cli/src/lib/signing/node-pop";

const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1"))
);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

/** Permanent owner-keyed Node. */
const NODE_SEED = Buffer.from("molpha_node");
const REGISTRY_SEED = Buffer.from("molpha_registry");
const SECP256K1_KEY_CLAIM_SEED = Buffer.from("molpha_secp_key");

const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const FEED_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");
const SELECTION_SEED_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_V1"))
);
const SELECTION_DOMAIN = keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE"));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;

const ACCOUNT_LAMPORTS = 1_000_000_000;
const REGISTERED_NODE_PDAS = new Map<string, PublicKey[]>();

export const EVM_FIXTURE_SOURCE_ID = hexToBytes(evmFixture.attestation.sourceId);
export const EVM_FIXTURE_VALUE = hexToBytes(evmFixture.attestation.value);
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
export function signerMaterialForVerifyAnswerEvm(): E2eSignerMaterial {
  return signerMaterialFromFixture(evmFixture as LoadedE2eFixture);
}

/** Matched secp256k1 key pairs for on-chain node registration tests. */
export function signerMaterialForNodeRegistration(
  nodeCount: number
): E2eSignerMaterial {
  const fixture = evmFixture as LoadedE2eFixture;
  const privKeys = fixturePrivkeys(fixture).slice(0, nodeCount);
  const compressedPubkeys = fixture.nodePubkeys
    .slice(0, nodeCount)
    .map((h) => hexToBytes(h));
  if (privKeys.length !== compressedPubkeys.length) {
    throw new Error(
      `signerMaterialForNodeRegistration: requested ${nodeCount} nodes but fixture has ${privKeys.length} keys`
    );
  }
  return { privKeys, compressedPubkeys };
}

/** Freshly signed args for `verify-answer-evm.json` fields (no `configHash` in message hash). */
export function buildSignedVerifyAnswerEvmArgs(
  registryVersion: number = evmFixture.attestation.registryVersion
) {
  return signE2eAttestation(
    registryVersion,
    signerMaterialForVerifyAnswerEvm(),
    evmUint256BitmapForFirstN(evmFixture.attestation.signaturesRequired),
    EVM_FIXTURE_SOURCE_ID,
    {
      value: EVM_FIXTURE_VALUE,
      canonicalTimestamp: evmFixture.attestation.canonicalTimestamp,
    },
    evmFixture.attestation.signaturesRequired
  ).submitArgs;
}

/** Static fixture args (registry version 1); signs at call time for on-chain e2e. */
export function buildEvmSubmitAttestationArgs() {
  return buildSignedVerifyAnswerEvmArgs(evmFixture.attestation.registryVersion);
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

function u8be(x: number): Uint8Array {
  const buf = new Uint8Array(1);
  buf[0] = x;
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
  sourceId: Uint8Array,
  registryVersion: number,
  canonicalTimestamp: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      SELECTION_SEED_PREFIX,
      Buffer.from(sourceId),
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

export function deriveSignersBitmap(
  sourceId: Uint8Array,
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
  const seed = deriveSelectionSeed(
    sourceId,
    registryVersion,
    canonicalTimestamp
  );
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
        u32be(signerCount),
        u32be(i),
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

export type E2eAttestationFields = {
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
  const redundancyBuffer = 2;
  const signerCount = fixture.attestation.signaturesRequired + redundancyBuffer;
  const privKeys = fixturePrivkeys(fixture).slice(0, signerCount);
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
  const signerCount = fixture.attestation.signaturesRequired;
  if (fixture.registryVersion !== fixture.attestation.registryVersion) {
    throw new Error("registryVersion != attestation.registryVersion");
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
  const signerCount = fixture.attestation.signaturesRequired;
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
    sourceId: hexToBytes(fixture.attestation.sourceId),
    registryVersion: fixture.attestation.registryVersion,
    signaturesRequired: fixture.attestation.signaturesRequired,
    signersBitmap,
    value: hexToBytes(fixture.attestation.value),
    canonicalTimestamp: fixture.attestation.canonicalTimestamp,
  });
}

/** Off-chain Schnorr-recovery check (same math as on-chain `verify_attestation`). */
export function verifyFixtureSchnorrSignature(fixture: LoadedE2eFixture): void {
  const material = fixture.secretKeys?.length
    ? signerMaterialFromFixture(fixture)
    : signerMaterialForVerifyAnswerEvm();
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  const signed = signE2eAttestation(
    fixture.attestation.registryVersion,
    material,
    signersBitmap,
    hexToBytes(fixture.attestation.sourceId),
    {
      value: hexToBytes(fixture.attestation.value),
      canonicalTimestamp: fixture.attestation.canonicalTimestamp,
    },
    fixture.attestation.signaturesRequired
  );
  const coalition = coalitionCompressedFromFixture(fixture);
  const messageHash = computeFixtureMessageHash(fixture);
  const { recoveryId, ecdsaSig, ecdsaHash } = evmSchnorrEcdsaInputs({
    coalitionPubkeyCompressed: coalition,
    messageHash,
    signature: signed.aggSigS,
    commitment: signed.commitment,
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
    `0x${Buffer.from(signed.commitment).toString("hex")}`
  ) {
    throw new Error("Schnorr recovery address mismatch");
  }
}

/** Anchor `submitAttestation` / `verifyAttestation` args from an e2e JSON fixture. */
export function buildSubmitArgsFromE2eFixture(fixture: LoadedE2eFixture) {
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  const material = fixture.secretKeys?.length
    ? signerMaterialFromFixture(fixture)
    : signerMaterialForVerifyAnswerEvm();
  return signE2eAttestation(
    fixture.attestation.registryVersion,
    material,
    signersBitmap,
    hexToBytes(fixture.attestation.sourceId),
    {
      value: hexToBytes(fixture.attestation.value),
      canonicalTimestamp: fixture.attestation.canonicalTimestamp,
    },
    fixture.attestation.signaturesRequired
  ).submitArgs;
}

function computeMessageHash(params: {
  sourceId: Uint8Array;
  registryVersion: number;
  signaturesRequired: number;
  signersBitmap: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
}): Uint8Array {
  return keccakConcat([
    MESSAGE_PREFIX,
    params.value,
    params.sourceId,
    u32be(params.registryVersion),
    u8be(params.signaturesRequired),
    u64be(params.canonicalTimestamp),
    params.signersBitmap,
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
  /**
   * Pin the Schnorr nonce `k`. Omit for a fresh random nonce. Reusing one `k` across two
   * signings forces both to share `commitment` (= address(k*G), a function of the nonce
   * alone) while each stays individually valid over its own message — the exact shape
   * `punish_nonce_reuse` prosecutes.
   */
  nonce?: bigint;
}): { s: Uint8Array; commitment: Uint8Array; nonce: bigint } {
  const px = params.coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = params.coalitionPubkeyCompressed[0] & 1;

  let k: bigint;
  if (params.nonce !== undefined) {
    k = modN(params.nonce);
    if (k === 0n) {
      throw new Error("schnorrSignForVerifier: nonce must be non-zero mod n");
    }
  } else {
    k = 0n;
    while (k === 0n) {
      k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
    }
  }
  const r = Point.BASE.multiply(k);
  const commitment = commitmentAddrFromPoint(r);

  const challengeHash = keccakConcat([
    px,
    Uint8Array.from([recoveryId]),
    params.messageHash,
    commitment,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + e * modN(params.coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitment, nonce: k };
}

export type SignedE2eAttestation = {
  messageHash: Uint8Array;
  submitArgs: ReturnType<typeof buildEvmSubmitAttestationArgs>;
  aggSigS: Uint8Array;
  commitment: Uint8Array;
  signersBitmap: Uint8Array;
  /** The Schnorr nonce `k` actually used. Feed back as `nonce` to force a commitment collision. */
  nonce: bigint;
};

/** Build submit args + signature material for the live registry coalition. */
export function signE2eAttestation(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCountOrBitmap: number | Uint8Array,
  sourceId: Uint8Array,
  attestation: E2eAttestationFields,
  signaturesRequired?: number,
  nodeCount?: number,
  redundancyBuffer = 2,
  nonce?: bigint
): SignedE2eAttestation {
  const selectionTimestamp = attestation.canonicalTimestamp;
  const signersBitmap =
    typeof signerCountOrBitmap === "number"
      ? deriveSignersBitmap(
          sourceId,
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
    sourceId,
    registryVersion,
    signaturesRequired: required,
    signersBitmap,
    value: attestation.value,
    canonicalTimestamp: attestation.canonicalTimestamp,
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
  const {
    s,
    commitment,
    nonce: usedNonce,
  } = schnorrSignForVerifier({
    coalitionPubkeyCompressed: coalitionCompressed,
    messageHash,
    coalitionPrivkey: coalitionSk,
    nonce,
  });

  const submitArgs = {
    sourceId: Array.from(sourceId),
    registryVersion,
    signaturesRequired: required,
    signersBitmap: Array.from(signersBitmap),
    value: Buffer.from(attestation.value),
    canonicalTimestamp: new BN(attestation.canonicalTimestamp),
    aggSigR: Array.from(new Uint8Array(32)),
    aggSigS: Array.from(s),
    commitment: Array.from(commitment),
  };

  return {
    messageHash,
    submitArgs,
    aggSigS: s,
    commitment,
    signersBitmap,
    nonce: usedNonce,
  };
}

/** On-chain `verify_attestation` return buffer length (`verify_attestation.rs`). */
export const VERIFY_DATA_UPDATE_RETURN_LEN = 72;

/** Pack expected return data: `value[32] | timestamp_be[8] | round_id[32]`. */
export function expectedVerifyAttestationReturnBytes(
  value: Uint8Array,
  canonicalTimestamp: number
): Uint8Array {
  const out = new Uint8Array(VERIFY_DATA_UPDATE_RETURN_LEN);
  out.set(value.subarray(0, 32), 0);
  out.set(u64be(canonicalTimestamp), 32);
  return out;
}

/**
 * Run `verify_attestation` via bankrun `tryProcessTransaction` and return program return data.
 */
export async function executeVerifyAttestationReturnData(
  ctx: TestContext,
  program: Program,
  signed: SignedE2eAttestation,
  _accounts: Record<string, never> = {},
  computeUnitLimit = 1_400_000
): Promise<Uint8Array> {
  const verifyIx = await program.methods
    .verifyAttestation(signed.submitArgs)
    .accountsPartial({
      registry: findRegistryPda(
        program.programId,
        signed.submitArgs.registryVersion
      ),
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
    throw new Error(`verify_attestation failed: ${result.result}\n${logs}`);
  }
  const returnData = result.meta?.returnData;
  if (!returnData) {
    throw new Error("verify_attestation did not set return data");
  }
  if (!returnData.programId.equals(program.programId)) {
    throw new Error(
      `return data program id mismatch: ${returnData.programId.toBase58()}`
    );
  }
  return new Uint8Array(returnData.data);
}

/** Build submit args signed for the live registry version and registered coalition. */
export function buildSignedEvmSubmitAttestationArgs(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCount = 8,
  sourceId: Uint8Array = EVM_FIXTURE_SOURCE_ID,
  attestation: E2eAttestationFields = {
    value: EVM_FIXTURE_VALUE,
    canonicalTimestamp: evmFixture.attestation.canonicalTimestamp,
  },
  nodeCount = signerCount,
  redundancyBuffer = 2
): ReturnType<typeof buildEvmSubmitAttestationArgs> {
  return signE2eAttestation(
    registryVersion,
    signerMaterial,
    signerCount,
    sourceId,
    attestation,
    signerCount,
    nodeCount,
    redundancyBuffer
  ).submitArgs;
}

/** Permanent owner-keyed Node PDA. */
export function findPermanentNodePda(
  programId: PublicKey,
  nodeOwner: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, nodeOwner.toBuffer()],
    programId
  )[0];
}

export const findNodePdaByOwner = findPermanentNodePda;

/** Permanent secp256k1 key reservation PDA. */
export function findSecp256k1KeyClaimPda(
  programId: PublicKey,
  secp256k1PubkeyCompressed: Uint8Array
): PublicKey {
  const fingerprint = keccak_256(secp256k1PubkeyCompressed);
  return PublicKey.findProgramAddressSync(
    [SECP256K1_KEY_CLAIM_SEED, Buffer.from(fingerprint)],
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
  popSigR: Uint8Array;
  popSigS: Uint8Array;
  ip?: number[];
  port?: number;
  securityDeposit?: BN;
}) {
  return {
    newRegistryVersion: params.newIndex + 1,
    registrationSalt: Array.from(params.registrationSalt),
    ip: params.ip ?? [127, 0, 0, 1],
    port: params.port ?? 8080 + params.newIndex,
    secp256K1PubkeyCompressed: Array.from(params.secp256k1PubkeyCompressed),
    secp256K1KeyFingerprint: Array.from(
      keccak_256(params.secp256k1PubkeyCompressed)
    ),
    popSigR: Array.from(params.popSigR),
    popSigS: Array.from(params.popSigS),
  };
}

/** Protocol-config-owned ATA used as the shared treasury. */
export function getProtocolTreasury(
  protocolConfigPda: PublicKey,
  usdcMint: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, protocolConfigPda, true);
}

function e2ePopNonce(programId: PublicKey, index: number): Uint8Array {
  return keccak_256(
    Buffer.concat([
      Buffer.from("molpha-e2e-pop-nonce-v1"),
      programId.toBuffer(),
      u32be(index),
    ])
  );
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
  signerMaterial: E2eSignerMaterial,
  advanceClock: (ctx: TestContext, seconds: number) => Promise<void>,
  opts?: { startIndex?: number; finalize?: boolean }
): Promise<void> {
  const startIndex = opts?.startIndex ?? 0;

  if (
    signerMaterial.privKeys.length !== signerMaterial.compressedPubkeys.length
  ) {
    throw new Error("registerNodesForE2e: private/public key count mismatch");
  }

  REGISTERED_NODE_PDAS.set(program.programId.toBase58(), []);

  for (let i = 0; i < signerMaterial.compressedPubkeys.length; i++) {
    const index = startIndex + i;
    if (!ctx.nodes[index]) {
      throw new Error(
        `registerNodesForE2e: missing node owner keypair at index ${index} (have ${ctx.nodes.length})`
      );
    }
    let secpCompressed = signerMaterial.compressedPubkeys[i];
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

    const registrationSalt = Uint8Array.from(
      Buffer.alloc(32, ((index + 1) % 255) + 1)
    );
    const pop = signNodePop({
      programId: program.programId,
      nodeId: deriveNodeId(nodeOwner, secpCompressed, registrationSalt),
      secp256k1PrivateKey: bytes32FromBigintBE(signerMaterial.privKeys[i]),
      nonce: e2ePopNonce(program.programId, index),
    });
    secpCompressed = pop.secp256k1PubkeyCompressed;
    signerMaterial.compressedPubkeys[i] = secpCompressed;
    const nodeId = deriveNodeId(nodeOwner, secpCompressed, registrationSalt);
    const nodePda = findNodePda(program.programId, nodeOwner);
    const registered =
      REGISTERED_NODE_PDAS.get(program.programId.toBase58()) ?? [];
    registered[index] = nodePda;
    REGISTERED_NODE_PDAS.set(program.programId.toBase58(), registered);
    const currentRegistryPda = findRegistryPda(program.programId, index);
    const newRegistryPda = findRegistryPda(program.programId, index + 1);
    const keyClaimPda = findSecp256k1KeyClaimPda(
      program.programId,
      secpCompressed
    );

    const registerNodeArgs = buildRegisterNodeArgs({
      newIndex: index,
      registrationSalt,
      secp256k1PubkeyCompressed: secpCompressed,
      popSigR: pop.popSigR,
      popSigS: pop.popSigS,
    });

    await program.methods
      .registerNode(registerNodeArgs)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        nodeOwner,
        protocolConfig: accounts.protocolConfigPda,
        registryState: accounts.registryStatePda,
        currentRegistry: currentRegistryPda,
        newRegistry: newRegistryPda,
        node: nodePda,
        secp256K1KeyClaim: keyClaimPda,
        usdcMint: accounts.usdcMint,
        ownerUsdc,
        treasury: accounts.treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 450_000 }),
      ])
      .signers([nodeOwnerKp])
      .rpc();
  }

  void advanceClock;
  void opts?.finalize;
}

export function registeredNodePda(
  programId: PublicKey,
  index: number
): PublicKey {
  const node = REGISTERED_NODE_PDAS.get(programId.toBase58())?.[index];
  if (!node)
    throw new Error(`No registered Node PDA recorded for index ${index}`);
  return node;
}

export function deriveFeedId(
  owner: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      FEED_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired]),
    ])
  );
}

/** On-chain feed id for the EVM fixture config hash and a given owner. */
export function deriveEvmFixtureFeedId(owner: PublicKey): Uint8Array {
  return deriveFeedId(
    owner,
    EVM_FIXTURE_SOURCE_ID,
    evmFixture.attestation.signaturesRequired
  );
}

export function findFeedPda(
  programId: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  owner: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      FEED_SEED,
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired]),
      Buffer.from(owner.toBuffer()),
    ],
    programId
  )[0];
}

/** Permanent owner-keyed Node PDA. */
export function findNodePda(programId: PublicKey, owner: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, owner.toBuffer()],
    programId
  )[0];
}

export function findRegistryPda(
  programId: PublicKey,
  version: number
): PublicKey {
  const versionBytes = Buffer.alloc(4);
  versionBytes.writeUInt32LE(version);
  return PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, versionBytes],
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

/** One permanent Node PDA per set bit, in ascending snapshot-bit order. */
export function signerRemainingAccounts(
  programId: PublicKey,
  signersBitmap: number[] | Uint8Array
) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit++) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    remaining.push({
      pubkey: registeredNodePda(programId, bit),
      isSigner: false,
      isWritable: false,
    });
  }
  return remaining;
}
