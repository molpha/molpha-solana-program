import { keccak_256 } from "@noble/hashes/sha3.js";

const DOMAIN_EQUIVOCATION = Buffer.from("MOLPHA_EQUIVOCATION_V1");
const DOMAIN_COMMITMENT_REUSE = Buffer.from("MOLPHA_COMMITMENT_REUSE_V1");
const DOMAIN_UNSOLICITED_SIGNING = Buffer.from("MOLPHA_UNSOLICITED_SIGNING_V1");
const DOMAIN_NONCE_REUSE = Buffer.from("MOLPHA_NONCE_REUSE_V1");
const DOMAIN_INACTIVITY = Buffer.from("MOLPHA_INACTIVITY_V2");

export function u32Be(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value >>> 0, 0);
  return out;
}

export function u64Be(value: number | bigint | string): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt(value), 0);
  return out;
}

export function pick<T = unknown>(
  obj: Record<string, unknown> | null | undefined,
  ...keys: string[]
): T | undefined {
  if (!obj) return undefined;
  for (const key of keys) {
    if (obj[key] !== undefined) return obj[key] as T;
  }
  return undefined;
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Missing ${label}`);
  }
  return value as Record<string, unknown>;
}

export function bytesField(
  value: unknown,
  label: string,
  expectedLength?: number,
): Uint8Array {
  let bytes: Uint8Array;
  if (typeof value === "string") {
    const clean = value.startsWith("0x") ? value.slice(2) : value;
    if (!/^[0-9a-fA-F]*$/.test(clean) || clean.length % 2 !== 0) {
      throw new Error(`Invalid ${label}`);
    }
    bytes = Uint8Array.from(Buffer.from(clean, "hex"));
  } else if (Array.isArray(value)) {
    bytes = Uint8Array.from(value as number[]);
  } else if (value instanceof Uint8Array) {
    bytes = value;
  } else {
    throw new Error(`Missing ${label}`);
  }
  if (expectedLength !== undefined && bytes.length !== expectedLength) {
    throw new Error(
      `${label} must be ${expectedLength} bytes, got ${bytes.length}`,
    );
  }
  return bytes;
}

function numField(value: unknown, label: string): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "bigint") return Number(value);
  if (typeof value === "string" && value !== "") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  throw new Error(`Missing ${label}`);
}

function attestationHash(attestation: unknown, label: string): Uint8Array {
  const att = asRecord(attestation, label);
  const payload = asRecord(pick(att, "payload"), `${label}.payload`);
  const signature = asRecord(pick(att, "signature"), `${label}.signature`);
  const signaturesRequired = numField(
    pick(payload, "signaturesRequired", "signatures_required"),
    `${label}.signaturesRequired`,
  );
  return keccak_256(
    Buffer.concat([
      Buffer.from(bytesField(pick(payload, "value"), `${label}.value`, 32)),
      Buffer.from(
        bytesField(
          pick(payload, "sourceId", "source_id"),
          `${label}.sourceId`,
          32,
        ),
      ),
      u32Be(
        numField(
          pick(payload, "registryVersion", "registry_version"),
          `${label}.registryVersion`,
        ),
      ),
      Buffer.from([signaturesRequired & 0xff]),
      u64Be(
        pick(payload, "canonicalTimestamp", "canonical_timestamp") as
          number | string | bigint,
      ),
      Buffer.from(
        bytesField(
          pick(signature, "aggSigS", "agg_sig_s"),
          `${label}.aggSigS`,
          32,
        ),
      ),
      Buffer.from(
        bytesField(pick(signature, "commitment"), `${label}.commitment`, 20),
      ),
      Buffer.from(
        bytesField(
          pick(signature, "signersBitmap", "signers_bitmap"),
          `${label}.signersBitmap`,
          32,
        ),
      ),
    ]),
  );
}

function canonicalPair(
  hashA: Uint8Array,
  hashB: Uint8Array,
): [Uint8Array, Uint8Array] {
  return Buffer.compare(Buffer.from(hashA), Buffer.from(hashB)) <= 0
    ? [hashA, hashB]
    : [hashB, hashA];
}

function pairEvidenceHash(
  domain: Buffer,
  pair: Record<string, unknown>,
  label: string,
): Uint8Array {
  const a = pick(pair, "attestationA", "attestation_a");
  const b = pick(pair, "attestationB", "attestation_b");
  const [lo, hi] = canonicalPair(
    attestationHash(a, `${label}.attestationA`),
    attestationHash(b, `${label}.attestationB`),
  );
  return keccak_256(Buffer.concat([domain, Buffer.from(lo), Buffer.from(hi)]));
}

function transcriptHash(value: unknown, label: string): Uint8Array {
  const transcript = asRecord(value, label);
  return keccak_256(
    Buffer.concat([
      Buffer.from(bytesField(pick(transcript, "b"), `${label}.b`, 32)),
      Buffer.from(bytesField(pick(transcript, "c"), `${label}.c`, 32)),
      Buffer.from(bytesField(pick(transcript, "s"), `${label}.s`, 32)),
    ]),
  );
}

/**
 * Inactivity case hash: `(epoch, roster_slot)` only, so a node is punished at most once per
 * epoch however its registry location changes.
 */
export function inactivityEvidenceHash(
  epoch: number | bigint,
  rosterSlot: number,
): Uint8Array {
  const slot = Buffer.alloc(2);
  slot.writeUInt16BE(rosterSlot, 0);
  return keccak_256(Buffer.concat([DOMAIN_INACTIVITY, u64Be(epoch), slot]));
}

/** Bits set in a Molpha `Bitmap` (`U256` little-endian bit positions, big-endian bytes). */
export function bitmapBits(bytes: Uint8Array): number[] {
  let n = 0n;
  for (const byte of bytes) {
    n = (n << 8n) | BigInt(byte);
  }
  const bits: number[] = [];
  for (let pos = 0; pos < 256; pos++) {
    if ((n >> BigInt(pos)) & 1n) bits.push(pos);
  }
  return bits;
}

export function unionBits(a: number[], b: number[]): number[] {
  return [...new Set([...a, ...b])].sort((x, y) => x - y);
}

export function offenceVariant(
  offence: Record<string, unknown>,
): Record<string, unknown> & { kind: string } {
  const inactivity = pick<Record<string, unknown>>(offence, "inactivity");
  if (inactivity) return { ...inactivity, kind: "inactivity" };
  const nonceReuse = pick<Record<string, unknown>>(
    offence,
    "nonceReuse",
    "nonce_reuse",
  );
  if (nonceReuse) return { ...nonceReuse, kind: "nonceReuse" };
  const equivocation = pick<Record<string, unknown>>(offence, "equivocation");
  if (equivocation) return { ...equivocation, kind: "equivocation" };
  const commitmentReuse = pick<Record<string, unknown>>(
    offence,
    "commitmentReuse",
    "commitment_reuse",
  );
  if (commitmentReuse) return { ...commitmentReuse, kind: "commitmentReuse" };
  const unsolicited = pick<Record<string, unknown>>(
    offence,
    "unsolicitedSigning",
    "unsolicited_signing",
  );
  if (unsolicited) return { ...unsolicited, kind: "unsolicitedSigning" };
  throw new Error("Unrecognised offence variant in --args");
}

export function registryVersionFromOffence(
  offence: Record<string, unknown>,
): number {
  const variant = offenceVariant(offence);
  if (variant.kind === "inactivity" || variant.kind === "nonceReuse") {
    return numField(
      pick(variant, "registryVersion", "registry_version"),
      "registry version",
    );
  }
  const attestation =
    variant.kind === "unsolicitedSigning"
      ? pick(variant, "attestation")
      : pick(variant, "attestationA", "attestation_a");
  const payload = asRecord(
    pick(asRecord(attestation, "attestation"), "payload"),
    "attestation.payload",
  );
  return numField(
    pick(payload, "registryVersion", "registry_version"),
    "registry version",
  );
}

/** `PunishmentCase` PDA seed: keccak of the offence evidence, matching on-chain `Evidence::hash`. */
export function offenceEvidenceHash(
  offence: Record<string, unknown>,
): Uint8Array {
  const variant = offenceVariant(offence);
  switch (variant.kind) {
    case "inactivity":
      return inactivityEvidenceHash(
        numField(pick(variant, "epoch"), "epoch"),
        numField(pick(variant, "rosterSlot", "roster_slot"), "roster slot"),
      );
    case "equivocation":
      return pairEvidenceHash(DOMAIN_EQUIVOCATION, variant, "equivocation");
    case "commitmentReuse":
      return pairEvidenceHash(
        DOMAIN_COMMITMENT_REUSE,
        variant,
        "commitmentReuse",
      );
    case "unsolicitedSigning":
      return keccak_256(
        Buffer.concat([
          DOMAIN_UNSOLICITED_SIGNING,
          Buffer.from(
            attestationHash(pick(variant, "attestation"), "attestation"),
          ),
        ]),
      );
    case "nonceReuse": {
      const nonces = asRecord(pick(variant, "nonces"), "nonceReuse.nonces");
      const [lo, hi] = canonicalPair(
        transcriptHash(
          pick(variant, "transcriptA", "transcript_a"),
          "transcriptA",
        ),
        transcriptHash(
          pick(variant, "transcriptB", "transcript_b"),
          "transcriptB",
        ),
      );
      return keccak_256(
        Buffer.concat([
          DOMAIN_NONCE_REUSE,
          u32Be(
            numField(
              pick(variant, "registryVersion", "registry_version"),
              "registry version",
            ),
          ),
          u32Be(
            numField(pick(variant, "nodeIndex", "node_index"), "node index"),
          ),
          u64Be(pick(variant, "epoch") as number | string | bigint),
          Buffer.from(
            bytesField(
              pick(variant, "batchMeta", "batch_meta"),
              "batchMeta",
              32,
            ),
          ),
          Buffer.from(
            bytesField(
              pick(variant, "batchRoot", "batch_root"),
              "batchRoot",
              32,
            ),
          ),
          u64Be(pick(variant, "slot") as number | string | bigint),
          Buffer.from(bytesField(pick(nonces, "r1X", "r1_x"), "r1X", 32)),
          Buffer.from(bytesField(pick(nonces, "r1Y", "r1_y"), "r1Y", 32)),
          Buffer.from(bytesField(pick(nonces, "r2X", "r2_x"), "r2X", 32)),
          Buffer.from(bytesField(pick(nonces, "r2Y", "r2_y"), "r2Y", 32)),
          Buffer.from(lo),
          Buffer.from(hi),
        ]),
      );
    }
    default:
      throw new Error("Unrecognised offence variant in --args");
  }
}

export function remainingBitsFromOffence(
  offence: Record<string, unknown>,
): number[] {
  const variant = offenceVariant(offence);
  if (variant.kind === "inactivity" || variant.kind === "nonceReuse") {
    return [numField(pick(variant, "nodeIndex", "node_index"), "node index")];
  }
  if (variant.kind === "unsolicitedSigning") {
    const attestation = asRecord(pick(variant, "attestation"), "attestation");
    const signature = asRecord(
      pick(attestation, "signature"),
      "attestation.signature",
    );
    return bitmapBits(
      bytesField(
        pick(signature, "signersBitmap", "signers_bitmap"),
        "signers bitmap",
        32,
      ),
    );
  }
  const a = asRecord(
    pick(variant, "attestationA", "attestation_a"),
    "attestationA",
  );
  const b = asRecord(
    pick(variant, "attestationB", "attestation_b"),
    "attestationB",
  );
  const sigA = asRecord(pick(a, "signature"), "attestationA.signature");
  const sigB = asRecord(pick(b, "signature"), "attestationB.signature");
  return unionBits(
    bitmapBits(
      bytesField(
        pick(sigA, "signersBitmap", "signers_bitmap"),
        "signers bitmap a",
        32,
      ),
    ),
    bitmapBits(
      bytesField(
        pick(sigB, "signersBitmap", "signers_bitmap"),
        "signers bitmap b",
        32,
      ),
    ),
  );
}

/**
 * Signer bitmaps an offence needs coalition keys for, keyed by the (camelCase) key field.
 *
 * Inactivity verifies no aggregate, and nonce reuse checks a single node's batch signature
 * against that node's own stored key, so neither carries one.
 */
export function offenceCoalitionBitmaps(
  offence: Record<string, unknown>,
): Array<[string, Uint8Array]> {
  const variant = offenceVariant(offence);
  const bitmapOf = (attestation: unknown, label: string) => {
    const signature = asRecord(
      pick(asRecord(attestation, label), "signature"),
      `${label}.signature`,
    );
    return bytesField(
      pick(signature, "signersBitmap", "signers_bitmap"),
      `${label} signers bitmap`,
      32,
    );
  };
  switch (variant.kind) {
    case "unsolicitedSigning":
      return [
        ["coalitionKey", bitmapOf(pick(variant, "attestation"), "attestation")],
      ];
    case "equivocation":
    case "commitmentReuse":
      return [
        [
          "coalitionKeyA",
          bitmapOf(
            pick(variant, "attestationA", "attestation_a"),
            "attestationA",
          ),
        ],
        [
          "coalitionKeyB",
          bitmapOf(
            pick(variant, "attestationB", "attestation_b"),
            "attestationB",
          ),
        ],
      ];
    default:
      return [];
  }
}

export function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  return (
    a.length === b.length &&
    Buffer.compare(Buffer.from(a), Buffer.from(b)) === 0
  );
}
