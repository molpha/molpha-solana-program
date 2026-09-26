import { BN } from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import * as crypto from "crypto";
import * as secp256k1 from "@noble/secp256k1";
import { Point } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3.js";
import {
  bytesToHex,
  fixedBytes,
  hexToBytes,
  parseBigIntString,
} from "../utils";
import { fixtureSourceId, VerifyAttestationFixture } from "./fixtures";

const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1")),
);
const SELECTION_SEED_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_V1")),
);
/** `keccak256("MOLPHA_SELECTION_DERIVE")` — EVM `NodeGroupBitmapLib.SELECTION_DOMAIN`. */
const SELECTION_DOMAIN = keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE"));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;
const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
);
export const VERIFY_ATTESTATION_RETURN_LEN = 72;
/** `Feed::MAX_VALUE_LEN` — upper bound on a `raw_value` preimage. */
export const MAX_RAW_VALUE_LEN = 256;

export type E2eSignerMaterial = {
  privKeys: bigint[];
  compressedPubkeys: Uint8Array[];
};

export type E2eAttestationFields = {
  value: Uint8Array;
  canonicalTimestamp: number;
  /** Preimage of `value`; submitted as `raw_value` so the feed stores `FeedValueKind::Hash`. */
  rawValue?: Uint8Array;
};

/** Aggregate signature parts as raw bytes (`SchnorrSignature` on chain). */
export type SchnorrSignatureBytes = {
  signersBitmap: Uint8Array;
  aggSigS?: Uint8Array;
  commitment?: Uint8Array;
};

/** Anchor `Attestation` instruction argument. */
export type AttestationArgs = {
  payload: {
    value: number[];
    sourceId: number[];
    registryVersion: number;
    signaturesRequired: number;
    canonicalTimestamp: BN;
  };
  signature: {
    aggSigS: number[];
    commitment: number[];
    signersBitmap: number[];
  };
};

/** Anchor `CoalitionKey`: affine sum of the signers' secp256k1 keys, big-endian. */
export type CoalitionKeyArgs = { x: number[]; y: number[] };

/**
 * Anchor `SubmitAttestationArgs`. `coalitionKey` may be left out while building offline; `submit`
 * then sums it from the signers' `Node` accounts.
 */
export type SubmitAttestationArgs = {
  attestation: AttestationArgs;
  rawValue: Buffer | null;
  coalitionKey?: CoalitionKeyArgs;
};

export type SignedAttestation = {
  submitArgs: SubmitAttestationArgs;
  signersBitmap: Uint8Array;
  aggSigS: Uint8Array;
  commitment: Uint8Array;
};

/**
 * Build the Anchor `Attestation` arg. Omitted signature parts are zeroed for callers that only
 * need to construct or serialize a payload; on-chain verification rejects them.
 */
export function buildAttestation(
  fields: {
    value: Uint8Array;
    sourceId: Uint8Array;
    registryVersion: number;
    signaturesRequired: number;
    canonicalTimestamp: number | BN;
  } & SchnorrSignatureBytes,
): AttestationArgs {
  if (fields.signaturesRequired < 0 || fields.signaturesRequired > 0xff) {
    throw new Error(
      `signatures required must fit in u8, got ${fields.signaturesRequired}`,
    );
  }
  return {
    payload: {
      value: Array.from(fixedBytes(fields.value, 32, "value")),
      sourceId: Array.from(fixedBytes(fields.sourceId, 32, "source id")),
      registryVersion: fields.registryVersion,
      signaturesRequired: fields.signaturesRequired,
      canonicalTimestamp: new BN(fields.canonicalTimestamp.toString()),
    },
    signature: {
      aggSigS: Array.from(
        fixedBytes(fields.aggSigS ?? new Uint8Array(32), 32, "agg sig s"),
      ),
      commitment: Array.from(
        fixedBytes(fields.commitment ?? new Uint8Array(20), 20, "commitment"),
      ),
      signersBitmap: Array.from(
        fixedBytes(fields.signersBitmap, 32, "signers bitmap"),
      ),
    },
  };
}

/** Anchor `raw_value` arg (`null` when absent). */
export function rawValueArg(rawValue: Uint8Array | undefined): Buffer | null {
  if (!rawValue) {
    return null;
  }
  if (rawValue.length > MAX_RAW_VALUE_LEN) {
    throw new Error(
      `raw value must be <= ${MAX_RAW_VALUE_LEN} bytes, got ${rawValue.length}`,
    );
  }
  return Buffer.from(rawValue);
}

function modN(x: bigint): bigint {
  const r = x % SECP256K1_ORDER_N;
  return r >= 0n ? r : r + SECP256K1_ORDER_N;
}

function bigintFromBytesBE(bytes: Uint8Array): bigint {
  return BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
}

function bytes32FromBigintBE(value: bigint): Uint8Array {
  let normalized = value;
  if (normalized < 0n) {
    normalized =
      ((normalized % SECP256K1_ORDER_N) + SECP256K1_ORDER_N) %
      SECP256K1_ORDER_N;
  }
  normalized %= SECP256K1_ORDER_N;
  return Uint8Array.from(
    Buffer.from(normalized.toString(16).padStart(64, "0"), "hex"),
  );
}

function keccakConcat(chunks: Uint8Array[]): Uint8Array {
  return keccak_256(Buffer.concat(chunks.map((chunk) => Buffer.from(chunk))));
}

function u32be(value: number): Uint8Array {
  const out = new Uint8Array(4);
  new DataView(out.buffer).setUint32(0, value >>> 0, false);
  return out;
}

function u64be(value: number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(value), false);
  return out;
}

function u256BeFromU64(counter: number): Uint8Array {
  const out = new Uint8Array(32);
  new DataView(out.buffer).setBigUint64(24, BigInt(counter), false);
  return out;
}

function selectionHashRound(seed: Uint8Array, counter: number): Uint8Array {
  return keccak_256(
    Buffer.concat([
      Buffer.from(seed),
      Buffer.from(SELECTION_DOMAIN),
      Buffer.from(u256BeFromU64(counter)),
    ]),
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
  groupSize: number,
): bigint {
  const limit = BigInt(Math.floor(0xffffffff / nodeCount) * nodeCount);
  let bitmap = 0n;
  let selected = 0;
  let counter = 0;
  while (selected < groupSize) {
    if (counter >= DERIVE_GROUP_BITMAP_MAX_ROUNDS) {
      throw new Error("deriveGroupBitmap exceeded Keccak-256 round limit");
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

function modReduceOnce32(hash: Uint8Array): Uint8Array {
  return bytes32FromBigintBE(bigintFromBytesBE(hash));
}

function isZero32(bytes: Uint8Array): boolean {
  return bytes.every((byte) => byte === 0);
}

function subBE256(a: Uint8Array, b: Uint8Array): Uint8Array {
  const av = bigintFromBytesBE(a);
  const bv = bigintFromBytesBE(b);
  const mod = 1n << 256n;
  return bytes32FromBigintBE((av - bv + mod) % mod);
}

function mulMod256(a: Uint8Array, b: Uint8Array): Uint8Array {
  const mod = 1n << 256n;
  const av = bigintFromBytesBE(a) % mod;
  const bv = bigintFromBytesBE(b) % mod;
  return bytes32FromBigintBE((av * bv) % mod);
}

export function evmUint256BitmapForFirstN(count: number): Uint8Array {
  const bitmap = new Uint8Array(32);
  for (let index = 0; index < count; index += 1) {
    const byteIndex = 31 - (index >> 3);
    bitmap[byteIndex] |= 1 << (index & 7);
  }
  return bitmap;
}

export function evmBitmapBitSet(
  signersBitmap: number[] | Uint8Array,
  bit: number,
): boolean {
  const byteIndex = 31 - Math.floor(bit / 8);
  const bitInByte = bit % 8;
  const byte = signersBitmap[byteIndex] ?? 0;
  return ((byte >> bitInByte) & 1) === 1;
}

export function bitmapPopcountEvm(signersBitmap: Uint8Array): number {
  let count = 0;
  for (let bit = 0; bit < 256; bit += 1) {
    if (evmBitmapBitSet(signersBitmap, bit)) {
      count += 1;
    }
  }
  return count;
}

/** Port of `NodeGroupBitmapLib.derive` / `derive_group_bitmap` in the on-chain program. */
export function deriveGroupBitmap(
  seed: Uint8Array,
  nodeCount: number,
  groupSize: number,
): Uint8Array {
  if (nodeCount <= 0) {
    throw new Error("nodeCount must be > 0");
  }
  if (nodeCount > 256) {
    throw new Error("nodeCount cannot exceed 256");
  }
  if (groupSize > nodeCount) {
    throw new Error("groupSize cannot exceed nodeCount");
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
  canonicalTimestamp: number,
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      SELECTION_SEED_PREFIX,
      Buffer.from(sourceId),
      Buffer.from(u32be(registryVersion)),
      Buffer.from(u64be(canonicalTimestamp)),
    ]),
  );
}

/** Mirrors `effective_selection_size` in `programs/molpha/src/utils/bitmap.rs`. */
export function effectiveSelectionSize(
  signaturesRequired: number,
  redundancyBuffer: number,
  nodeCount: number,
): number {
  return Math.min(nodeCount, signaturesRequired + redundancyBuffer);
}

export function deriveSignersBitmap(
  sourceId: Uint8Array,
  registryVersion: number,
  nodeCount: number,
  signaturesRequired: number,
  redundancyBuffer = 2,
  canonicalTimestamp = 0,
): Uint8Array {
  const selectionSize = effectiveSelectionSize(
    signaturesRequired,
    redundancyBuffer,
    nodeCount,
  );
  return deriveGroupBitmap(
    deriveSelectionSeed(sourceId, registryVersion, canonicalTimestamp),
    nodeCount,
    selectionSize,
  );
}

export function generateE2eSignerMaterialFromSeed(
  seed: string,
  signerCount: number,
): E2eSignerMaterial {
  const privKeys: bigint[] = [];
  const compressedPubkeys: Uint8Array[] = [];

  for (let index = 0; index < signerCount; index += 1) {
    const skSeed = keccak_256(
      Buffer.concat([
        Buffer.from(seed),
        Buffer.from([signerCount]),
        Buffer.from([index]),
      ]),
    );
    const ecdh = crypto.createECDH("secp256k1");
    ecdh.setPrivateKey(Buffer.from(skSeed));
    const compressed = ecdh.getPublicKey(undefined, "compressed");
    privKeys.push(bigintFromBytesBE(skSeed));
    compressedPubkeys.push(Uint8Array.from(compressed));
  }

  return { privKeys, compressedPubkeys };
}

/** Mirrors `molpha_verifier::compute_message_hash`. */
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
    Uint8Array.from([params.signaturesRequired]),
    u64be(params.canonicalTimestamp),
    params.signersBitmap,
  ]);
}

function commitmentAddrFromPoint(point: Point): Uint8Array {
  const uncompressed = point.toRawBytes(false);
  const digest = keccak_256(uncompressed.subarray(1));
  return digest.subarray(12, 32);
}

function schnorrSignForVerifier(params: {
  coalitionPubkeyCompressed: Uint8Array;
  messageHash: Uint8Array;
  coalitionPrivkey: bigint;
}): { s: Uint8Array; commitment: Uint8Array } {
  const px = params.coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = params.coalitionPubkeyCompressed[0] & 1;
  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const commitmentPoint = Point.BASE.multiply(k);
  const commitment = commitmentAddrFromPoint(commitmentPoint);
  const challengeHash = keccakConcat([
    px,
    Uint8Array.from([recoveryId]),
    params.messageHash,
    commitment,
  ]);
  const challenge = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + challenge * modN(params.coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitment };
}

export function signE2eAttestation(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCountOrBitmap: number | Uint8Array,
  sourceId: Uint8Array,
  attestation: E2eAttestationFields,
  signaturesRequired?: number,
): SignedAttestation {
  const signersBitmap =
    typeof signerCountOrBitmap === "number"
      ? evmUint256BitmapForFirstN(signerCountOrBitmap)
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
  let coalitionSecretKey = 0n;
  for (let bit = 0; bit < 256; bit += 1) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    if (bit >= signerMaterial.privKeys.length) {
      throw new Error(`Missing signer material for registry index ${bit}`);
    }
    coalitionSecretKey = modN(
      coalitionSecretKey + signerMaterial.privKeys[bit],
    );
    coalitionPoint = coalitionPoint.add(
      Point.fromHex(
        Buffer.from(signerMaterial.compressedPubkeys[bit]).toString("hex"),
      ),
    );
  }

  const coalitionCompressed = coalitionPoint.toRawBytes(true);
  const coalitionAffine = coalitionPoint.toAffine();
  const { s, commitment } = schnorrSignForVerifier({
    coalitionPubkeyCompressed: coalitionCompressed,
    messageHash,
    coalitionPrivkey: coalitionSecretKey,
  });

  return {
    submitArgs: {
      attestation: buildAttestation({
        value: attestation.value,
        sourceId,
        registryVersion,
        signaturesRequired: required,
        canonicalTimestamp: attestation.canonicalTimestamp,
        signersBitmap,
        aggSigS: s,
        commitment,
      }),
      rawValue: rawValueArg(attestation.rawValue),
      coalitionKey: {
        x: Array.from(secp256k1.etc.numberToBytesBE(coalitionAffine.x)),
        y: Array.from(secp256k1.etc.numberToBytesBE(coalitionAffine.y)),
      },
    },
    signersBitmap,
    aggSigS: s,
    commitment,
  };
}

export function buildSubmitArgsFromFixture(
  fixture: VerifyAttestationFixture,
): SubmitAttestationArgs {
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap),
  );
  const signaturesRequired =
    fixture.dataUpdate.signaturesRequired ?? bitmapPopcountEvm(signersBitmap);
  return {
    attestation: buildAttestation({
      value: hexToBytes(fixture.dataUpdate.value, 32, "value"),
      sourceId: hexToBytes(fixtureSourceId(fixture), 32, "source id"),
      registryVersion: fixture.dataUpdate.registryVersion,
      signaturesRequired,
      canonicalTimestamp: fixture.dataUpdate.canonicalTimestamp,
      signersBitmap,
      aggSigS: hexToBytes(fixture.schnorrSignature.signature, 32, "signature"),
      commitment: hexToBytes(
        fixture.schnorrSignature.commitment.toLowerCase(),
        20,
        "commitment",
      ),
    }),
    rawValue: null,
  };
}

function uint256BE(value: bigint): Uint8Array {
  const out = new Uint8Array(32);
  let next = value;
  for (let index = 31; index >= 0 && next > 0n; index -= 1) {
    out[index] = Number(next & 0xffn);
    next >>= 8n;
  }
  return out;
}

export function expectedVerifyAttestationReturnBytes(
  value: Uint8Array,
  canonicalTimestamp: number,
): Uint8Array {
  const out = new Uint8Array(VERIFY_ATTESTATION_RETURN_LEN);
  out.set(value.subarray(0, 32), 0);
  out.set(u64be(canonicalTimestamp), 32);
  return out;
}

export function signerRemainingAccounts(
  registry: { nodeCount: number; nodes: Array<number[] | Uint8Array> },
  signersBitmap: number[] | Uint8Array,
  isWritable = false,
) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit += 1) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }

    if (bit >= Number(registry.nodeCount)) {
      throw new Error(`signer bit ${bit} is outside registry node_count`);
    }
    remaining.push({
      pubkey: new PublicKey(Uint8Array.from(registry.nodes[bit])),
      isSigner: false,
      isWritable,
    });
  }
  return remaining;
}

/** Node accounts required by immediate settlement: verified signers first, then selected misses. */
export function settlementRemainingAccounts(
  registry: {
    version: number;
    nodeCount: number;
    redundancyBuffer: number;
    nodes: Array<number[] | Uint8Array>;
  },
  payload: {
    sourceId: number[] | Uint8Array;
    registryVersion: number;
    signaturesRequired: number;
    canonicalTimestamp: number | BN;
  },
  signersBitmap: number[] | Uint8Array,
) {
  const selection = deriveSignersBitmap(
    Uint8Array.from(payload.sourceId),
    payload.registryVersion,
    Number(registry.nodeCount),
    payload.signaturesRequired,
    Number(registry.redundancyBuffer),
    Number(payload.canonicalTimestamp.toString()),
  );
  const signerBits: number[] = [];
  const missedBits: number[] = [];
  for (let bit = 0; bit < Number(registry.nodeCount); bit += 1) {
    const signed = evmBitmapBitSet(signersBitmap, bit);
    const selected = evmBitmapBitSet(selection, bit);
    if (signed && !selected) {
      throw new Error(`signer bit ${bit} is outside the derived selection`);
    }
    if (signed) signerBits.push(bit);
    else if (selected) missedBits.push(bit);
  }
  for (let bit = Number(registry.nodeCount); bit < 256; bit += 1) {
    if (evmBitmapBitSet(signersBitmap, bit)) {
      throw new Error(`signer bit ${bit} is outside the registry`);
    }
  }
  return [...signerBits, ...missedBits].map((bit) => ({
    pubkey: new PublicKey(Uint8Array.from(registry.nodes[bit])),
    isSigner: false,
    isWritable: true,
  }));
}

export function signerMaterialFromFixture(
  fixture: VerifyAttestationFixture,
): E2eSignerMaterial {
  const compressedPubkeys = fixture.nodePubkeys.map((value) =>
    hexToBytes(value, 33, "node pubkey"),
  );
  const scalarStrings = fixture.secretKeys ?? fixture.nodePrivkeys;
  if (!scalarStrings || scalarStrings.length === 0) {
    throw new Error("Fixture must contain secretKeys or nodePrivkeys");
  }

  const privKeys = scalarStrings.map((value) =>
    value.startsWith("0x")
      ? bigintFromBytesBE(hexToBytes(value, 32, "node privkey"))
      : parseBigIntString(value, "secret key"),
  );

  return { privKeys, compressedPubkeys };
}

export function describeSignedPayload(signed: SignedAttestation) {
  return {
    submitArgs: signed.submitArgs,
    signersBitmapHex: bytesToHex(signed.signersBitmap),
    aggSigSHex: bytesToHex(signed.aggSigS),
    commitmentAddrHex: bytesToHex(signed.commitment),
  };
}

export function buildBitmapFromFlag(value: string): Uint8Array {
  return hexToBytes(value, 32, "signers bitmap");
}

const SECP256K1_ORDER_BYTES = Uint8Array.from(
  Buffer.from(
    "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
    "hex",
  ),
);

export function evmSchnorrEcdsaInputs(params: {
  coalitionPubkeyCompressed: Uint8Array;
  messageHash: Uint8Array;
  signature: Uint8Array;
  commitment: Uint8Array;
}): { recoveryId: number; ecdsaSig: Uint8Array; ecdsaHash: Uint8Array } {
  const parityPrefix = params.coalitionPubkeyCompressed[0];
  if (parityPrefix !== 0x02 && parityPrefix !== 0x03) {
    throw new Error("invalid compressed pubkey prefix");
  }

  const recoveryId = parityPrefix & 1;
  const px = params.coalitionPubkeyCompressed.subarray(1, 33);
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
  const sigReduced = modReduceOnce32(params.signature);
  const msgMul = mulMod256(sigReduced, pxModN);
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
