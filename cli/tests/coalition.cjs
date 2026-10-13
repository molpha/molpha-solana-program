const assert = require("node:assert/strict");
const { test } = require("node:test");
const { etc, Point, CURVE } = require("@noble/secp256k1");
const {
  deriveSignersBitmap,
  generateE2eSignerMaterialFromSeed,
  signE2eAttestation,
} = require("../dist/lib/signing/attestation");
const {
  coalitionKeyForSigners,
  coalitionKeyFromPoint,
  withOffenceCoalitionKeys,
} = require("../dist/lib/coalition");
const {
  bitmapBits,
  offenceCoalitionBitmaps,
  remainingBitsFromOffence,
} = require("../dist/lib/slash");

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
      timestamp: 1_700_000_000_000,
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
      timestamp: 1_700_000_000_000,
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

test("supplied offence keys are parsed in place and never summed from the registry", () => {
  // An empty registry: summing the signers' keys from it would throw, so a filled key proves
  // the supplied one was used.
  const registry = { nodeCount: 0, members: [] };
  const offence = {
    equivocation: {
      attestationA: attestation(1),
      attestationB: attestation(2),
      coalition_key_a: { x: `0x${"11".repeat(32)}`, y: bytes(32, 0x22) },
      coalitionKeyB: { x: bytes(32, 0x33), y: `0x${"44".repeat(32)}` },
    },
  };
  const filled = withOffenceCoalitionKeys(registry, offence);
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

/** A registry snapshot whose member `i` signs with private key `i + 1`. */
function snapshot(count) {
  const members = [];
  const points = [];
  for (let i = 0; i < count; i += 1) {
    const point = Point.BASE.multiply(BigInt(i + 1));
    const { x, y } = point.toAffine();
    points.push(point);
    members.push({
      node: bytes(32, i + 1),
      x: Array.from(etc.numberToBytesBE(x)),
      y: Array.from(etc.numberToBytesBE(y)),
    });
  }
  return { registry: { nodeCount: count, members }, points };
}

test("a coalition key is summed from the snapshot's member keys, no node accounts needed", () => {
  const { registry, points } = snapshot(4);
  const bitmap = bytes(32, 0);
  bitmap[31] = 0b1011; // members 0, 1 and 3
  const key = coalitionKeyForSigners(registry, bitmap);
  const expected = points[0].add(points[1]).add(points[3]);
  assert.deepEqual(key, coalitionKeyFromPoint(expected));

  // A signer outside the snapshot, or no signer at all, has no key.
  const outside = bytes(32, 0);
  outside[31] = 0b10000;
  assert.throws(() => coalitionKeyForSigners(registry, outside), /outside node_count/);
  assert.throws(() => coalitionKeyForSigners(registry, bytes(32, 0)), /empty/);
});

test("missing offence keys are filled from the evidence registry", () => {
  const { registry, points } = snapshot(3);
  const filled = withOffenceCoalitionKeys(registry, {
    unsolicitedSigning: { attestation: attestation(0b101) },
  });
  assert.deepEqual(
    filled.unsolicitedSigning.coalitionKey,
    coalitionKeyFromPoint(points[0].add(points[2])),
  );
});

test("slash accounts are the guilty nodes only", () => {
  const registry = { nodeCount: 8, redundancyBuffer: 2 };
  // Equivocation and commitment reuse convict the signers both attestations share.
  const pair = {
    attestationA: attestation(0b00111100),
    attestationB: attestation(0b01111000),
  };
  assert.deepEqual(
    remainingBitsFromOffence({ equivocation: pair }, registry),
    [3, 4, 5],
  );
  assert.deepEqual(
    remainingBitsFromOffence({ commitmentReuse: pair }, registry),
    [3, 4, 5],
  );

  // Nonce reuse and inactivity name one node.
  assert.deepEqual(
    remainingBitsFromOffence({ nonceReuse: { nodeIndex: 6 } }, registry),
    [6],
  );
  assert.deepEqual(
    remainingBitsFromOffence(
      { inactivity: { registryVersion: 1, nodeIndex: 2 } },
      registry,
    ),
    [2],
  );

  // Unsolicited signing convicts the signers outside the derived selection.
  const duty = attestation(0);
  const selection = bitmapBits(
    deriveSignersBitmap(
      Uint8Array.from(duty.payload.sourceId),
      duty.payload.registryVersion,
      registry.nodeCount,
      duty.payload.signaturesRequired,
      registry.redundancyBuffer,
      duty.payload.timestamp,
    ),
  );
  const outsider = [0, 1, 2, 3, 4, 5, 6, 7].find((bit) => !selection.includes(bit));
  const signed = bytes(32, 0);
  signed[31] = (1 << outsider) | (1 << selection[0]);
  const guilty = remainingBitsFromOffence(
    {
      unsolicitedSigning: {
        attestation: {
          ...duty,
          signature: { ...duty.signature, signersBitmap: signed },
        },
      },
    },
    registry,
  );
  assert.deepEqual(guilty, [outsider]);
});
