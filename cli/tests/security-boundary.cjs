const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const { BN } = require("@coral-xyz/anchor");
const {
  buildAttestation,
  rawValueArg,
  buildBitmapFromFlag,
} = require("../dist/lib/signing/attestation");
const { buildCorpus, integer } = require("./lib/security-vectors.cjs");

test("CLI encoders and signatures reproduce the corpus executed by Rust/SBF", () => {
  const fixture = JSON.parse(
    fs.readFileSync(
      path.join(
        __dirname,
        "../../molpha-tests/tests/fixtures/security-boundary.json",
      ),
      "utf8",
    ),
  );
  assert.deepEqual(buildCorpus(), fixture);
});

test("BN timestamps retain every bit around 2^53 and u64 max", () => {
  for (const value of [
    "9007199254740991",
    "9007199254740992",
    "9007199254740993",
    "18446744073709551615",
  ]) {
    const args = buildAttestation({
      value: new Uint8Array(32),
      sourceId: new Uint8Array(32),
      registryVersion: 0xffffffff,
      signaturesRequired: 3,
      timestamp: new BN(value),
      signersBitmap: new Uint8Array(32),
    });
    assert.equal(args.payload.timestamp.toString(), value);
    assert.equal(
      integer(args.payload.timestamp.toString(), 8)
        .readBigUInt64LE()
        .toString(),
      value,
    );
  }
});

test("raw values and full-width bitmap parsing preserve exact boundaries", () => {
  assert.equal(rawValueArg(undefined), null);
  assert.equal(rawValueArg(new Uint8Array(0)).length, 0);
  assert.equal(rawValueArg(new Uint8Array(256)).length, 256);
  assert.throws(() => rawValueArg(new Uint8Array(257)), /256/);
  for (const bit of [0, 63, 64, 65, 255]) {
    const value = (1n << BigInt(bit)).toString(16).padStart(64, "0");
    const parsed = buildBitmapFromFlag(`0x${value}`);
    assert.equal(Buffer.from(parsed).toString("hex"), value);
  }
});
