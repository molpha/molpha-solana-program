/**
 * TS mirrors of `state::punishment::equivocation_evidence_hash` /
 * `state::punishment::unsolicited_signing_evidence_hash` and the `PunishmentCase` PDA, plus small
 * bitmap set-ops used by node-punishment tests.
 */
import { keccak_256 } from "@noble/hashes/sha3";
import { PublicKey } from "@solana/web3.js";
import { evmBitmapBitSet } from "./submit-attestation";

const DOMAIN_EQUIVOCATION = Buffer.from("MOLPHA_EQUIVOCATION_V1");
const DOMAIN_UNSOLICITED_SIGNING = Buffer.from("MOLPHA_UNSOLICITED_SIGNING_V1");
const DOMAIN_NONCE_REUSE = Buffer.from("MOLPHA_NONCE_REUSE_V1");
const PUNISHMENT_CASE_SEED = Buffer.from("molpha_punishment");

/** One signed statement offered as punishment evidence. Mirrors `state::punishment::SignedStatement`. */
export type SignedStatementBytes = {
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

function statementHash(statement: SignedStatementBytes): Buffer {
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
  statementA: SignedStatementBytes;
  statementB: SignedStatementBytes;
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

/** Mirrors `state::punishment::unsolicited_signing_evidence_hash`. */
export function unsolicitedSigningEvidenceHash(params: {
  sourceId: Uint8Array;
  registryVersion: number;
  signaturesRequired: number;
  canonicalTimestamp: number;
  statement: SignedStatementBytes;
}): Buffer {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        DOMAIN_UNSOLICITED_SIGNING,
        Buffer.from(params.sourceId),
        u32be(params.registryVersion),
        Buffer.from([params.signaturesRequired]),
        u64beFromNumber(params.canonicalTimestamp),
        statementHash(params.statement),
      ])
    )
  );
}

/** Mirrors `state::punishment::SigningContext`. */
export type SigningContextFields = {
  sourceId: Uint8Array;
  signaturesRequired: number;
  canonicalTimestamp: number;
};

/** Mirrors `state::punishment::context_statement_hash`. */
function contextStatementHash(
  context: SigningContextFields,
  statement: SignedStatementBytes
): Buffer {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        Buffer.from(context.sourceId),
        Buffer.from([context.signaturesRequired]),
        u64beFromNumber(context.canonicalTimestamp),
        statementHash(statement),
      ])
    )
  );
}

/** Mirrors `state::punishment::nonce_reuse_evidence_hash`. */
export function nonceReuseEvidenceHash(params: {
  registryVersion: number;
  contextA: SigningContextFields;
  statementA: SignedStatementBytes;
  contextB: SigningContextFields;
  statementB: SignedStatementBytes;
}): Buffer {
  const hashA = contextStatementHash(params.contextA, params.statementA);
  const hashB = contextStatementHash(params.contextB, params.statementB);
  const [lo, hi] =
    Buffer.compare(hashA, hashB) <= 0 ? [hashA, hashB] : [hashB, hashA];
  return Buffer.from(
    keccak_256(
      Buffer.concat([DOMAIN_NONCE_REUSE, u32be(params.registryVersion), lo, hi])
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

/** `a & ~b` — bits set in `a` but not in `b`. */
export function differenceBitmap(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = a[i] & ~b[i];
  return out;
}

export function bitsOfBitmap(bitmap: Uint8Array): number[] {
  const bits: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (evmBitmapBitSet(bitmap, bit)) bits.push(bit);
  }
  return bits;
}
