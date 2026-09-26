const assert = require("node:assert/strict");
const { test } = require("node:test");
const { etc, Point, CURVE } = require("@noble/secp256k1");
const {
  generateE2eSignerMaterialFromSeed,
  signE2eAttestation,
} = require("../dist/lib/signing/attestation");
const {
  coalitionKeyFromPoint,
  withOffenceCoalitionKeys,
} = require("../dist/lib/coalition");
const { offenceCoalitionBitmaps } = require("../dist/lib/slash");

const bytes = (length, fill) => Array(length).fill(fill);

function attestation(bitmapByte) {
  const signersBitmap = bytes(32, 0);
  signersBitmap[31] = bitmapByte;
  return {
    payload: {
      value: bytes(32, 1),
      sourceId: bytes(32, 2),
      registryVersion: 3,
      signaturesRequired: 2,
      canonicalTimestamp: 1_700_000_000,
    },
    signature: {
      aggSigS: bytes(32, 4),
      commitment: bytes(20, 5),
      signersBitmap,
    },
  };
}

test("signed submit args carry the signers' affine key sum", () => {
  const material = generateE2eSignerMaterialFromSeed("coalition-key", 5);
  const signed = signE2eAttestation(
    3,
    material,
    5,
    Uint8Array.from(bytes(32, 7)),
    {
      value: Uint8Array.from(bytes(32, 9)),
      canonicalTimestamp: 1_700_000_000,
    },
  );

  // Independent derivation: (Σ sk)·G rather than Σ (sk·G).
  const secret = material.privKeys.reduce(
    (acc, sk) => (acc + sk) % CURVE.n,
    0n,
  );
  const expected = Point.BASE.multiply(secret).toAffine();
  assert.deepEqual(signed.submitArgs.coalitionKey, {
    x: Array.from(etc.numberToBytesBE(expected.x)),
    y: Array.from(etc.numberToBytesBE(expected.y)),
  });
  assert.throws(() => coalitionKeyFromPoint(Point.ZERO), /infinity/);
});

test("pair offences need two keys, unsolicited signing one, the rest none", () => {
  const pair = {
    attestationA: attestation(0b011),
    attestationB: attestation(0b110),
  };
  assert.deepEqual(
    offenceCoalitionBitmaps({ equivocation: pair }).map(([field, bitmap]) => [
      field,
      bitmap[31],
    ]),
    [
      ["coalitionKeyA", 0b011],
      ["coalitionKeyB", 0b110],
    ],
  );
  assert.deepEqual(
    offenceCoalitionBitmaps({ commitmentReuse: pair }).map(([field]) => field),
    ["coalitionKeyA", "coalitionKeyB"],
  );
  assert.deepEqual(
    offenceCoalitionBitmaps({
      unsolicitedSigning: { attestation: attestation(1) },
    }).map(([field]) => field),
    ["coalitionKey"],
  );
  assert.deepEqual(
    offenceCoalitionBitmaps({
      inactivity: { registryVersion: 1, nodeIndex: 0 },
    }),
    [],
  );
});

test("supplied offence keys are parsed in place and never fetched", async () => {
  const program = {
    account: {
      node: {
        fetchMultiple: () => {
          throw new Error("must not fetch supplied keys");
        },
      },
    },
  };
  const registry = { nodeCount: 0, nodes: [] };
  const offence = {
    equivocation: {
      attestationA: attestation(1),
      attestationB: attestation(2),
      coalition_key_a: { x: `0x${"11".repeat(32)}`, y: bytes(32, 0x22) },
      coalitionKeyB: { x: bytes(32, 0x33), y: `0x${"44".repeat(32)}` },
    },
  };
  const filled = await withOffenceCoalitionKeys(program, registry, offence);
  assert.deepEqual(filled.equivocation.coalitionKeyA, {
    x: bytes(32, 0x11),
    y: bytes(32, 0x22),
  });
  assert.deepEqual(filled.equivocation.coalitionKeyB, {
    x: bytes(32, 0x33),
    y: bytes(32, 0x44),
  });
  assert.equal(filled.equivocation.coalition_key_a, undefined);
});
