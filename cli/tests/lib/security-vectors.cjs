// Deterministic public test vectors. Nonces and tiny scalar keys must never be used operationally.
const fs = require("node:fs");
const { BN } = require("@coral-xyz/anchor");
const { Point, CURVE, etc } = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3.js");
const {
  buildAttestation,
  computeMessageHash,
  schnorrSignForVerifier,
} = require("../../dist/lib/signing/attestation");

function integer(value, length) {
  const bytes = Buffer.alloc(length);
  let n = BigInt(value);
  for (let i = 0; i < length; i += 1) {
    bytes[i] = Number(n & 255n);
    n >>= 8n;
  }
  if (n !== 0n) throw new Error("integer exceeds wire width");
  return bytes;
}

function wire(attestation, raw, coalition) {
  const p = attestation.payload;
  const s = attestation.signature;
  return Buffer.concat([
    Buffer.from(p.value),
    Buffer.from(p.sourceId),
    integer(p.registryVersion, 4),
    integer(p.signaturesRequired, 1),
    integer(p.timestamp.toString(), 8),
    Buffer.from(s.aggSigS),
    Buffer.from(s.commitment),
    Buffer.from(s.signersBitmap),
    raw === null
      ? Buffer.from([0])
      : Buffer.concat([Buffer.from([1]), integer(raw.length, 4), raw]),
    Buffer.from(coalition.x),
    Buffer.from(coalition.y),
  ]);
}

function exactHash(p, bitmap) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        Buffer.from(keccak_256(Buffer.from("MOLPHA_MESSAGE_V1"))),
        Buffer.from(p.value),
        Buffer.from(p.sourceId),
        Buffer.from(integer(p.registryVersion, 4)).reverse(),
        integer(p.signaturesRequired, 1),
        Buffer.from(integer(p.timestamp.toString(), 8)).reverse(),
        bitmap,
      ]),
    ),
  );
}

function buildCorpus() {
  const specs = [
    ["nodes", "1700200000000", [0, 1, 2], null],
    ["provider", "1700200000001", [0, 1, 65], null],
    ["empty-preimage", "1700200000002", [0, 1, 2], Buffer.alloc(0)],
    ["32-byte-preimage", "1700200000003", [0, 1, 2], Buffer.alloc(32, 41)],
    ["256-byte-preimage", "1700200000004", [0, 1, 65], Buffer.alloc(256, 42)],
    ["safe-integer-max", "9007199254740991", [0, 1, 2], null],
    ["two-to-53", "9007199254740992", [0, 1, 2], null],
    ["bn-above-safe-integer", "9007199254740993", [0, 1, 65], null],
    ["u64-max", "18446744073709551615", [0, 1, 2], null],
  ];
  const cases = specs.map(([name, timestamp, bits, raw], index) => {
    const bitmap = Buffer.alloc(32);
    for (const bit of bits) bitmap[31 - (bit >> 3)] |= 1 << (bit & 7);
    const value =
      raw === null ? Buffer.alloc(32, index + 1) : Buffer.from(keccak_256(raw));
    const fields = {
      value,
      sourceId: Buffer.from(keccak_256(Buffer.from(`security-vector:${name}`))),
      registryVersion: 5,
      signaturesRequired: 3,
      timestamp: new BN(timestamp),
      signersBitmap: bitmap,
    };
    const unsigned = buildAttestation(fields);
    const hash = exactHash(unsigned.payload, bitmap);
    let secret = 0n;
    for (const bit of bits)
      secret = (secret + BigInt(bit === 65 ? 5 : bit + 1)) % CURVE.n;
    const point = Point.BASE.multiply(secret);
    const signature = schnorrSignForVerifier({
      coalitionPubkeyCompressed: point.toRawBytes(true),
      messageHash: hash,
      coalitionPrivkey: secret,
      nonce: etc.numberToBytesBE(BigInt(index + 101)),
    });
    const attestation = buildAttestation({
      ...fields,
      aggSigS: signature.s,
      commitment: signature.commitment,
    });
    const affine = point.toAffine();
    const coalition = {
      x: etc.numberToBytesBE(affine.x),
      y: etc.numberToBytesBE(affine.y),
    };
    if (BigInt(timestamp) <= BigInt(Number.MAX_SAFE_INTEGER)) {
      const clientHash = computeMessageHash({
        ...fields,
        timestamp: Number(timestamp),
      });
      if (!hash.equals(Buffer.from(clientHash)))
        throw new Error("number API differs from exact hash");
    }
    return {
      name,
      timestamp,
      message_hash: hash.toString("hex"),
      args_hex: wire(attestation, raw, coalition).toString("hex"),
      outcome: "ok",
    };
  });
  const tampered = {
    ...cases[0],
    name: "tampered-value",
    outcome: "InvalidAggregateSignature",
  };
  const bytes = Buffer.from(tampered.args_hex, "hex");
  bytes[0] ^= 1;
  tampered.args_hex = bytes.toString("hex");
  cases.push(tampered);
  const badKey = {
    ...cases[0],
    name: "tampered-coalition-key",
    outcome: "InvalidCoalitionKey",
  };
  const keyBytes = Buffer.from(badKey.args_hex, "hex");
  keyBytes[keyBytes.length - 1] ^= 1;
  badKey.args_hex = keyBytes.toString("hex");
  cases.push(badKey);
  return {
    format: 1,
    provenance:
      "Deterministic JS CLI encoders and Schnorr signer; Rust independently decodes, hashes, registers keys and executes SBF.",
    cases,
  };
}

module.exports = { buildCorpus, integer };
if (require.main === module) {
  if (process.argv[2] !== "--write" || !process.argv[3])
    throw new Error("usage: node security-vectors.cjs --write <path>");
  fs.writeFileSync(
    process.argv[3],
    `${JSON.stringify(buildCorpus(), null, 2)}\n`,
  );
}
