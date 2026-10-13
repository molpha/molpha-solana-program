const assert = require("node:assert/strict");
const { test } = require("node:test");
const { PublicKey } = require("@solana/web3.js");
const {
  buildKeyRotationArgs,
  deriveKeyRotationDigest,
  signKeyRotation,
} = require("../dist/lib/signing/key-rotation");

const hex = (bytes) => Buffer.from(bytes).toString("hex");
const scalar = (n) => {
  const out = new Uint8Array(32);
  out[31] = n;
  return out;
};

// Shared with `molpha-verifier` `pop::tests::rotation_vector_is_pinned`, which pins the same
// bytes from an independent Rust signer: program `[1; 32]`, node `[3; 32]`, current secret 9,
// new secret 10, aggregate nonce 17, PoP nonce 21.
const PROGRAM_ID = new PublicKey(new Uint8Array(32).fill(1));
const NODE = new PublicKey(new Uint8Array(32).fill(3));
const VECTOR = {
  digest: "331b9a9c19c4c7f72c68d960fdd661892fa15c49e6f82d9ae3a23265e4be751c",
  newPublicKey:
    "03a0434d9e47f3c86235477c7b1ae6ae5d3442d49b1943c2b752a68e2a47e247c7",
  oldKeySigS:
    "4eb385ddc525b86e70a8aec7e4642b61b4f34312e1353527dcc0a440ad0423cf",
  oldKeyCommitment: "252dae0a4b9d9b80f504f6418acd2d364c0c59cd",
  popSigR: "352bbf4a4cdd12564f93fa332ce333301d9ad40271f8107181340aef25be59d5",
  popSigS: "6f5983bad23c650ba9cfb4b82920f073790f38c800b1c1dad98b756269f17009",
};

test("the rotation digest and both signatures match the Rust verifier's vector", () => {
  const proof = signKeyRotation({
    programId: PROGRAM_ID,
    node: NODE,
    currentPrivateKey: scalar(9),
    newPrivateKey: scalar(10),
    nonces: { current: scalar(17), pop: scalar(21) },
  });
  assert.equal(hex(proof.newPublicKeyCompressed), VECTOR.newPublicKey);
  assert.equal(
    hex(
      deriveKeyRotationDigest(
        PROGRAM_ID,
        NODE,
        Uint8Array.from(proof.newPublicKeyCompressed),
      ),
    ),
    VECTOR.digest,
  );
  assert.equal(hex(proof.oldKeySigS), VECTOR.oldKeySigS);
  assert.equal(hex(proof.oldKeyCommitment), VECTOR.oldKeyCommitment);
  assert.equal(hex(proof.popSigR), VECTOR.popSigR);
  assert.equal(hex(proof.popSigS), VECTOR.popSigS);
});

test("the digest binds the program, the node and the new key", () => {
  const key = Uint8Array.from(Buffer.from(VECTOR.newPublicKey, "hex"));
  const base = hex(deriveKeyRotationDigest(PROGRAM_ID, NODE, key));
  assert.equal(base, VECTOR.digest);

  const otherProgram = new PublicKey(new Uint8Array(32).fill(2));
  const otherNode = new PublicKey(new Uint8Array(32).fill(4));
  const otherKey = Uint8Array.from(key);
  otherKey[32] ^= 1;
  for (const changed of [
    deriveKeyRotationDigest(otherProgram, NODE, key),
    deriveKeyRotationDigest(PROGRAM_ID, otherNode, key),
    deriveKeyRotationDigest(PROGRAM_ID, NODE, otherKey),
  ]) {
    assert.notEqual(hex(changed), base);
  }
  assert.throws(
    () => deriveKeyRotationDigest(PROGRAM_ID, NODE, key.subarray(1)),
    /33 bytes/,
  );
});

test("rotation arguments are length-checked and shaped for Anchor", () => {
  const good = {
    newPublicKeyCompressed: new Uint8Array(33).fill(1),
    oldKeySigS: new Uint8Array(32).fill(2),
    oldKeyCommitment: new Uint8Array(20).fill(3),
    popSigR: new Uint8Array(32).fill(4),
    popSigS: new Uint8Array(32).fill(5),
  };
  const args = buildKeyRotationArgs(good);
  assert.deepEqual(Object.keys(args), [
    "newPublicKeyCompressed",
    "oldKeySigS",
    "oldKeyCommitment",
    "popSigR",
    "popSigS",
  ]);
  assert.equal(args.newPublicKeyCompressed.length, 33);
  assert.equal(args.oldKeyCommitment.length, 20);

  for (const [field, length] of [
    ["newPublicKeyCompressed", 32],
    ["oldKeySigS", 31],
    ["oldKeyCommitment", 32],
    ["popSigR", 33],
    ["popSigS", 0],
  ]) {
    assert.throws(
      () => buildKeyRotationArgs({ ...good, [field]: new Uint8Array(length) }),
      /must/,
      field,
    );
  }
});

test("invalid private keys are refused", () => {
  const base = {
    programId: PROGRAM_ID,
    node: NODE,
    currentPrivateKey: scalar(9),
    newPrivateKey: scalar(10),
  };
  assert.throws(() => signKeyRotation({ ...base, newPrivateKey: new Uint8Array(32) }), /invalid/);
  assert.throws(() => signKeyRotation({ ...base, currentPrivateKey: new Uint8Array(31) }), /32 bytes/);
});
