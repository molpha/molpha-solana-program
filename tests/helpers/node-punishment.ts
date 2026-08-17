/**
 * TS mirrors of `state::punishment::equivocation_evidence_hash` and the `PunishmentCase` PDA, plus
 * small bitmap set-ops used by node-punishment tests.
 */
import { keccak_256 } from "@noble/hashes/sha3";
import { PublicKey } from "@solana/web3.js";
import { evmBitmapBitSet } from "./submit-data-update";

const DOMAIN_EQUIVOCATION = Buffer.from("MOLPHA_EQUIVOCATION_V1");
const PUNISHMENT_CASE_SEED = Buffer.from("molpha_punishment");

export type EquivocationStatementBytes = {
  value: Uint8Array;
  aggSigS: Uint8Array;
  commitmentAddr: Uint8Array;
  signersBitmap: Uint8Array;
};

function u32be(x: number): Buffer {
  const buf = Buffer.alloc(4);
  buf.writeUInt32BE(x >>> 0, 0);
  return buf;
}

function u64beFromNumber(x: number): Buffer {
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64BE(BigInt(x), 0);
  return buf;
}

function statementHash(statement: EquivocationStatementBytes): Buffer {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        Buffer.from(statement.value),
        Buffer.from(statement.aggSigS),
        Buffer.from(statement.commitmentAddr),
        Buffer.from(statement.signersBitmap),
      ])
    )
  );
}

/** Mirrors `state::punishment::equivocation_evidence_hash`. */
export function equivocationEvidenceHash(params: {
  sourceId: Uint8Array;
  registryVersion: number;
  signaturesRequired: number;
  canonicalTimestamp: number;
  statementA: EquivocationStatementBytes;
  statementB: EquivocationStatementBytes;
}): Buffer {
  const hashA = statementHash(params.statementA);
  const hashB = statementHash(params.statementB);
  const [lo, hi] =
    Buffer.compare(hashA, hashB) <= 0 ? [hashA, hashB] : [hashB, hashA];
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        DOMAIN_EQUIVOCATION,
        Buffer.from(params.sourceId),
        u32be(params.registryVersion),
        Buffer.from([params.signaturesRequired]),
        u64beFromNumber(params.canonicalTimestamp),
        lo,
        hi,
      ])
    )
  );
}

export function findPunishmentCasePda(
  programId: PublicKey,
  evidenceHash: Uint8Array
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [PUNISHMENT_CASE_SEED, Buffer.from(evidenceHash)],
    programId
  )[0];
}

export function unionBitmap(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = a[i] | b[i];
  return out;
}

export function intersectionBitmap(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = a[i] & b[i];
  return out;
}

export function bitsOfBitmap(bitmap: Uint8Array): number[] {
  const bits: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (evmBitmapBitSet(bitmap, bit)) bits.push(bit);
  }
  return bits;
}
