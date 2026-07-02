import { BN } from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import * as crypto from "crypto";
import * as secp256k1 from "@noble/secp256k1";
import { Point } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
import { RemoveTransitionInfo } from "../accounts";
import { findNodePda } from "../pdas";
import { bytesToHex, hexToBytes, parseBigIntString } from "../utils";
import { VerifyDataUpdateFixture } from "./fixtures";

const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1"))
);
const SELECTION_SEED_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_SELECTION_V1"))
);
/** `keccak256("MOLPHA_SELECTION_DERIVE")` — EVM `NodeGroupBitmapLib.SELECTION_DOMAIN`. */
const SELECTION_DOMAIN = keccak_256(Buffer.from("MOLPHA_SELECTION_DERIVE"));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65_536;
const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);
export const VERIFY_DATA_UPDATE_RETURN_LEN = 72;
export const VIRTUAL_INDEX = 0xffffffff;

export type E2eSignerMaterial = {
  privKeys: bigint[];
  compressedPubkeys: Uint8Array[];
};

export type E2eDataUpdateFields = {
  configHash: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
};

export type SignedDataUpdate = {
  submitArgs: {
    jobId: number[];
    registryVersion: number;
    signersBitmap: number[];
    value: number[];
    canonicalTimestamp: BN;
    aggSigR: number[];
    aggSigS: number[];
    commitmentAddr: number[];
  };
  signersBitmap: Uint8Array;
  aggSigS: Uint8Array;
  commitmentAddr: Uint8Array;
};

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
    Buffer.from(normalized.toString(16).padStart(64, "0"), "hex")
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
  bit: number
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
  groupSize: number
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
  canonicalTimestamp = 0
): Uint8Array {
  const selectionSize = effectiveSelectionSize(
    signaturesRequired,
    redundancyBuffer,
    nodeCount
  );
  return deriveGroupBitmap(
    deriveSelectionSeed(jobId, registryVersion, canonicalTimestamp),
    nodeCount,
    selectionSize
  );
}

export function generateE2eSignerMaterialFromSeed(
  seed: string,
  signerCount: number
): E2eSignerMaterial {
  const privKeys: bigint[] = [];
  const compressedPubkeys: Uint8Array[] = [];

  for (let index = 0; index < signerCount; index += 1) {
    const skSeed = keccak_256(
      Buffer.concat([
        Buffer.from(seed),
        Buffer.from([signerCount]),
        Buffer.from([index]),
      ])
    );
    const ecdh = crypto.createECDH("secp256k1");
    ecdh.setPrivateKey(Buffer.from(skSeed));
    const compressed = ecdh.getPublicKey(undefined, "compressed");
    privKeys.push(bigintFromBytesBE(skSeed));
    compressedPubkeys.push(Uint8Array.from(compressed));
  }

  return { privKeys, compressedPubkeys };
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

function commitmentAddrFromPoint(point: Point): Uint8Array {
  const uncompressed = point.toRawBytes(false);
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
  const commitmentPoint = Point.BASE.multiply(k);
  const commitmentAddr = commitmentAddrFromPoint(commitmentPoint);
  const challengeHash = keccakConcat([
    px,
    Uint8Array.from([recoveryId]),
    params.messageHash,
    commitmentAddr,
  ]);
  const challenge = modN(bigintFromBytesBE(challengeHash));
  const s = modN(k + challenge * modN(params.coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitmentAddr };
}

export function signE2eDataUpdate(
  registryVersion: number,
  signerMaterial: E2eSignerMaterial,
  signerCountOrBitmap: number | Uint8Array,
  jobId: Uint8Array,
  dataUpdate: E2eDataUpdateFields,
  signaturesRequired?: number
): SignedDataUpdate {
  const signersBitmap =
    typeof signerCountOrBitmap === "number"
      ? evmUint256BitmapForFirstN(signerCountOrBitmap)
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
  let coalitionSecretKey = 0n;
  for (let bit = 0; bit < 256; bit += 1) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }
    if (bit >= signerMaterial.privKeys.length) {
      throw new Error(`Missing signer material for registry index ${bit}`);
    }
    coalitionSecretKey = modN(
      coalitionSecretKey + signerMaterial.privKeys[bit]
    );
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
    coalitionPrivkey: coalitionSecretKey,
  });

  return {
    submitArgs: {
      jobId: Array.from(jobId),
      registryVersion,
      signersBitmap: Array.from(signersBitmap),
      value: Array.from(dataUpdate.value),
      canonicalTimestamp: new BN(dataUpdate.canonicalTimestamp),
      aggSigR: Array.from(new Uint8Array(32)),
      aggSigS: Array.from(s),
      commitmentAddr: Array.from(commitmentAddr),
    },
    signersBitmap,
    aggSigS: s,
    commitmentAddr,
  };
}

export function buildSubmitArgsFromFixture(fixture: VerifyDataUpdateFixture) {
  const signersBitmap = uint256BE(
    BigInt(fixture.schnorrSignature.signersBitmap)
  );
  return {
    jobId: Array.from(hexToBytes(fixture.dataUpdate.jobId, 32, "job id")),
    registryVersion: fixture.dataUpdate.registryVersion,
    signersBitmap: Array.from(signersBitmap),
    value: Array.from(hexToBytes(fixture.dataUpdate.value, 32, "value")),
    canonicalTimestamp: new BN(fixture.dataUpdate.canonicalTimestamp),
    aggSigR: Array.from(new Uint8Array(32)),
    aggSigS: Array.from(
      hexToBytes(fixture.schnorrSignature.signature, 32, "signature")
    ),
    commitmentAddr: Array.from(
      hexToBytes(
        fixture.schnorrSignature.commitment.toLowerCase(),
        20,
        "commitment"
      )
    ),
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

export function expectedVerifyDataUpdateReturnBytes(
  value: Uint8Array,
  canonicalTimestamp: number
): Uint8Array {
  const out = new Uint8Array(VERIFY_DATA_UPDATE_RETURN_LEN);
  out.set(value.subarray(0, 32), 0);
  out.set(u64be(canonicalTimestamp), 32);
  return out;
}

export function signerRemainingAccounts(
  programId: PublicKey,
  signersBitmap: number[] | Uint8Array,
  transition?: RemoveTransitionInfo
) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit += 1) {
    if (!evmBitmapBitSet(signersBitmap, bit)) {
      continue;
    }

    const effectiveIndex = expectedRegistryIndex(bit, transition);
    remaining.push({
      pubkey: findNodePda(programId, effectiveIndex)[0],
      isSigner: false,
      isWritable: false,
    });
  }
  return remaining;
}

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

export function signerMaterialFromFixture(
  fixture: VerifyDataUpdateFixture
): E2eSignerMaterial {
  const compressedPubkeys = fixture.nodePubkeys.map((value) =>
    hexToBytes(value, 33, "node pubkey")
  );
  const scalarStrings = fixture.secretKeys ?? fixture.nodePrivkeys;
  if (!scalarStrings || scalarStrings.length === 0) {
    throw new Error("Fixture must contain secretKeys or nodePrivkeys");
  }

  const privKeys = scalarStrings.map((value) =>
    value.startsWith("0x")
      ? bigintFromBytesBE(hexToBytes(value, 32, "node privkey"))
      : parseBigIntString(value, "secret key")
  );

  return { privKeys, compressedPubkeys };
}

export function describeSignedPayload(signed: SignedDataUpdate) {
  return {
    submitArgs: {
      ...signed.submitArgs,
      canonicalTimestamp: signed.submitArgs.canonicalTimestamp.toString(),
    },
    signersBitmapHex: bytesToHex(signed.signersBitmap),
    aggSigSHex: bytesToHex(signed.aggSigS),
    commitmentAddrHex: bytesToHex(signed.commitmentAddr),
  };
}

export function buildBitmapFromFlag(value: string): Uint8Array {
  return hexToBytes(value, 32, "signers bitmap");
}

const SECP256K1_ORDER_BYTES = Uint8Array.from(
  Buffer.from(
    "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
    "hex"
  )
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
