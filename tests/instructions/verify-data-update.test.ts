/**
 * EVM-compatibility tests for `verify_data_update` / `submit_data_update`.
 *
 * These tests validate that the TypeScript-side message construction matches
 * what the Rust on-chain program computes, using the vectors from
 * `tests/fixtures-json/verify-answer-evm.json`.
 *
 * Full Schnorr-recovery verification (secp256k1 + keccak) is also covered in
 * the Rust unit-test suite (`programs/molpha/src/instructions/data_update_core.rs`).
 */

import { keccak_256 } from "@noble/hashes/sha3";
import fixture from "../fixtures-json/verify-answer-evm.json";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function hexToBytes(hex: string): Uint8Array {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(h.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/** Encode a u32 as 4 big-endian bytes. */
function u32BE(n: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setUint32(0, n, false);
  return buf;
}

/** Encode a u64 as 8 big-endian bytes (accepts number or bigint). */
function u64BE(n: number | bigint): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, BigInt(n), false);
  return buf;
}

/** Encode a uint256 integer as big-endian 32 bytes. */
function uint256BE(n: number | bigint): Uint8Array {
  const buf = new Uint8Array(32);
  let v = BigInt(n);
  for (let i = 31; i >= 0 && v > BigInt(0); i--) {
    buf[i] = Number(v & BigInt(0xff));
    v >>= BigInt(8);
  }
  return buf;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((s, p) => s + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
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

// ---------------------------------------------------------------------------
// Constants that must match the Rust program
// ---------------------------------------------------------------------------

/**
 * `EVM_ROUND_ID_PREFIX` = `bytes32(keccak256("MOLPHA_PULL_ROUND_V1"))`.
 *
 * Verified here as a JavaScript reference:
 *   keccak256(utf8("MOLPHA_PULL_ROUND_V1"))
 */
const EXPECTED_EVM_ROUND_ID_PREFIX = Buffer.from([
  0x05, 0xcc, 0x9a, 0xbf, 0xa8, 0x3c, 0xdc, 0x2d, 0xff, 0xa2, 0x8a, 0xe0,
  0xd4, 0x68, 0x59, 0x5e, 0xce, 0xd0, 0x56, 0x4a, 0x16, 0xf6, 0xed, 0x9a,
  0x3e, 0x2c, 0x5b, 0x4e, 0x65, 0x2d, 0x98, 0x26,
]);

// ---------------------------------------------------------------------------
// Fixture data (decoded once, used across tests)
// ---------------------------------------------------------------------------

const jobId = hexToBytes(fixture.dataUpdate.jobId);
const configHash = hexToBytes(fixture.dataUpdate.configHash);
const value = hexToBytes(fixture.dataUpdate.value);
const registryVersion: number = fixture.dataUpdate.registryVersion;
const signaturesRequired: number = fixture.dataUpdate.signaturesRequired;
const canonicalTimestamp: number = fixture.dataUpdate.canonicalTimestamp;
const signersBitmapInt = BigInt(fixture.schnorrSignature.signersBitmap);
const signersBitmap = uint256BE(signersBitmapInt);
const sigS = hexToBytes(fixture.schnorrSignature.signature);
const commitment = hexToBytes(
  fixture.schnorrSignature.commitment.toLowerCase()
);
const nodePubkeys = fixture.nodePubkeys.map((h) => hexToBytes(h));

// ---------------------------------------------------------------------------
// EVM_ROUND_ID_PREFIX
// ---------------------------------------------------------------------------

describe("EVM_ROUND_ID_PREFIX constant", () => {
  it("equals keccak256('MOLPHA_PULL_ROUND_V1')", () => {
    const computed = keccak_256(Buffer.from("MOLPHA_PULL_ROUND_V1"));
    expect(Buffer.from(computed)).toEqual(EXPECTED_EVM_ROUND_ID_PREFIX);
  });
});

// ---------------------------------------------------------------------------
// Message hash construction
// ---------------------------------------------------------------------------

describe("compute_message_hash (EVM Validator._constructMessage)", () => {
  /**
   * Replicates the Rust `compute_message_hash` logic:
   *
   * ```
   * keccak256(abi.encodePacked(
   *   EVM_ROUND_ID_PREFIX,
   *   jobId,
   *   registryVersion (uint32 BE),
   *   signaturesRequired (uint32 BE),
   *   configHash,
   *   signersBitmap (uint256 BE),
   *   value,
   *   canonicalTimestamp (uint64 BE)
   * ))
   * ```
   */
  function computeMessageHash(): Uint8Array {
    const preimage = concat(
      EXPECTED_EVM_ROUND_ID_PREFIX,
      jobId,
      u32BE(registryVersion),
      u32BE(signaturesRequired),
      configHash,
      signersBitmap,
      value,
      u64BE(canonicalTimestamp)
    );
    return keccak_256(preimage);
  }

  function buildMessagePreimage(): Uint8Array {
    return concat(
      EXPECTED_EVM_ROUND_ID_PREFIX,
      jobId,
      u32BE(registryVersion),
      u32BE(signaturesRequired),
      configHash,
      signersBitmap,
      value,
      u64BE(canonicalTimestamp)
    );
  }

  it("produces a 32-byte hash", () => {
    const hash = computeMessageHash();
    expect(hash.length).toBe(32);
  });

  it("is deterministic", () => {
    expect(computeMessageHash()).toEqual(computeMessageHash());
  });

  it("changes when registry_version is mutated", () => {
    const base = computeMessageHash();
    const altered = concat(
      EXPECTED_EVM_ROUND_ID_PREFIX,
      jobId,
      u32BE(registryVersion + 1),
      u32BE(signaturesRequired),
      configHash,
      signersBitmap,
      value,
      u64BE(canonicalTimestamp)
    );
    expect(keccak_256(altered)).not.toEqual(base);
  });

  it("changes when value is mutated", () => {
    const base = computeMessageHash();
    const mutValue = new Uint8Array(value);
    mutValue[0] ^= 0xff;
    const altered = concat(
      EXPECTED_EVM_ROUND_ID_PREFIX,
      jobId,
      u32BE(registryVersion),
      u32BE(signaturesRequired),
      configHash,
      signersBitmap,
      mutValue,
      u64BE(canonicalTimestamp)
    );
    expect(keccak_256(altered)).not.toEqual(base);
  });

  it("changes when signers_bitmap is mutated", () => {
    const base = computeMessageHash();
    const mutBitmap = new Uint8Array(signersBitmap);
    mutBitmap[31] ^= 0x01;
    const altered = concat(
      EXPECTED_EVM_ROUND_ID_PREFIX,
      jobId,
      u32BE(registryVersion),
      u32BE(signaturesRequired),
      configHash,
      mutBitmap,
      value,
      u64BE(canonicalTimestamp)
    );
    expect(keccak_256(altered)).not.toEqual(base);
  });

  it("encodes the message preimage with the exact expected byte length", () => {
    // 32 + 32 + 4 + 4 + 32 + 32 + 32 + 8
    expect(buildMessagePreimage().length).toBe(176);
  });

  it("matches a fixed known-good fixture hash vector", () => {
    const expectedHashHex =
      "288215b1e5ca1f32f13f2f7fb2b0337fcc82e2604ffd63178d78d2312feb2e39";
    expect(Buffer.from(computeMessageHash()).toString("hex")).toBe(
      expectedHashHex
    );
  });
});

// ---------------------------------------------------------------------------
// Fixture integrity
// ---------------------------------------------------------------------------

describe("verify-answer-evm.json fixture integrity", () => {
  it("fixture has exactly 10 node pubkeys", () => {
    expect(fixture.nodePubkeys.length).toBe(10);
  });

  it("all node pubkeys are 33 bytes (compressed secp256k1)", () => {
    for (const pk of nodePubkeys) {
      expect(pk.length).toBe(33);
      // Compressed point prefix must be 0x02 or 0x03.
      expect([0x02, 0x03]).toContain(pk[0]);
    }
  });

  it("signersBitmap 255 encodes exactly 8 set bits", () => {
    let count = 0;
    let v = signersBitmapInt;
    while (v > BigInt(0)) {
      count += Number(v & BigInt(1));
      v >>= BigInt(1);
    }
    expect(count).toBe(fixture.dataUpdate.signaturesRequired);
  });

  it("commitment is a 20-byte Ethereum address", () => {
    expect(commitment.length).toBe(20);
  });

  it("signature scalar s is 32 bytes", () => {
    expect(sigS.length).toBe(32);
    // Must not be all-zero.
    expect(sigS.some((b) => b !== 0)).toBe(true);
  });

  it("nodeIndexesOneBased covers the first 8 nodes (matching signersBitmap)", () => {
    // signersBitmap = 255 → bits 0..7 set (0-indexed) → nodes 1..8 (1-indexed)
    expect(fixture.nodeIndexesOneBased).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it("nodeIndexesOneBased is derived correctly from signersBitmap", () => {
    expect(bitmapToNodeIndexesOneBased(signersBitmapInt)).toEqual(
      fixture.nodeIndexesOneBased
    );
  });

  it("registryVersion in fixture matches schnorrSignature field context", () => {
    expect(fixture.registryVersion).toBe(fixture.dataUpdate.registryVersion);
  });
});

// ---------------------------------------------------------------------------
// Return-data layout
// ---------------------------------------------------------------------------

describe("verify_data_update return-data layout", () => {
  it("return buffer is 72 bytes: value[32] | timestamp_be[8] | round_id[32]", () => {
    // Simulate the Rust return-data packing.
    const returnData = new Uint8Array(72);
    returnData.set(value, 0);
    const tsBytes = u64BE(canonicalTimestamp);
    returnData.set(tsBytes, 32);
    // round_id is all-zero in this fixture (not part of the EVM signature)
    // returnData[40..72] stays zero.

    // Verify slices decode correctly.
    const recoveredValue = returnData.slice(0, 32);
    const recoveredTs = new DataView(returnData.buffer).getBigUint64(32, false);
    const recoveredRoundId = returnData.slice(40, 72);

    expect(Buffer.from(recoveredValue)).toEqual(Buffer.from(value));
    expect(recoveredTs).toBe(BigInt(canonicalTimestamp));
    expect(recoveredRoundId.every((b) => b === 0)).toBe(true);
  });
});
