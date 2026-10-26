const assert = require("node:assert/strict");
const { test } = require("node:test");
const {
  decodeRegistryMembers,
  registryExtendsNeeded,
  registrySpace,
  MAX_GROWTH_PER_INSTRUCTION,
  MAX_REGISTRY_NODES,
  REGISTRY_HEADER_LEN,
  REGISTRY_MEMBER_LEN,
} = require("../dist/lib/registry");
const {
  registryNode,
  signerMembers,
} = require("../dist/lib/signing/attestation");

/** Raw registry account bytes for `count` members, with `spare` unpopulated records after them. */
function registryData(count, spare = 0) {
  const data = new Uint8Array(registrySpace(count + spare));
  for (let index = 0; index < count; index += 1) {
    const offset = REGISTRY_HEADER_LEN + index * REGISTRY_MEMBER_LEN;
    data.fill(0x10 + index, offset, offset + 32); // node
    data.fill(0x40 + index, offset + 32, offset + 64); // x
    data.fill(0x80 + index, offset + 64, offset + 96); // y
  }
  return data;
}

// Shared with `molpha-tests/tests/encoding.rs::registry_layout_matches_the_verifier`.
test("a registry account is a 32-byte header and 96 bytes per member", () => {
  assert.equal(REGISTRY_HEADER_LEN, 32);
  assert.equal(REGISTRY_MEMBER_LEN, 96);
  assert.equal(registrySpace(0), 32);
  assert.equal(registrySpace(1), 128);
  assert.equal(registrySpace(106), 10_208);
  assert.equal(registrySpace(107), 10_304);
  assert.equal(registrySpace(MAX_REGISTRY_NODES), 24_608);
});

test("members are decoded from the raw account in registry order", () => {
  const members = decodeRegistryMembers(registryData(3), 3);
  assert.equal(members.length, 3);
  members.forEach((member, index) => {
    assert.deepEqual(member.node, new Uint8Array(32).fill(0x10 + index));
    assert.deepEqual(member.x, new Uint8Array(32).fill(0x40 + index));
    assert.deepEqual(member.y, new Uint8Array(32).fill(0x80 + index));
  });
  assert.deepEqual(decodeRegistryMembers(registryData(0), 0), []);

  // The decoded snapshot is what the bitmap helpers read.
  const registry = { nodeCount: 3, members };
  assert.deepEqual(
    registryNode(registry, 2).toBytes(),
    new Uint8Array(32).fill(0x12),
  );
  const bitmap = new Uint8Array(32);
  bitmap[31] = 0b101;
  assert.equal(signerMembers(registry, bitmap).length, 2);
});

test("a malformed registry account is refused", () => {
  // Shorter than a header, and a table that ends mid-record.
  assert.throws(() => decodeRegistryMembers(new Uint8Array(31), 0), /header/);
  assert.throws(
    () => decodeRegistryMembers(registryData(2).subarray(0, 200), 1),
    /whole member records/,
  );
  // A header that claims more members than the account holds.
  assert.throws(() => decodeRegistryMembers(registryData(2), 3), /claims 3/);
});

test("extend_registry is needed only past what one instruction can allocate", () => {
  assert.equal(MAX_GROWTH_PER_INSTRUCTION, 10_240);
  // Shared with `molpha-tests/tests/registry_allocation.rs`.
  for (const [members, extends_] of [
    [0, 0],
    [1, 0],
    [106, 0],
    [107, 1],
    [213, 1],
    [214, 2],
    [256, 2],
  ]) {
    assert.equal(registryExtendsNeeded(members), extends_, `${members} members`);
  }
});

test("whatever is already allocated for the next version counts", () => {
  // One earlier extension leaves 107 members within the publishing instruction's reach.
  assert.equal(registryExtendsNeeded(107, 10_240), 0);
  assert.equal(registryExtendsNeeded(214, 10_240), 1);
  assert.equal(registryExtendsNeeded(256, 20_480), 0);
  // An account already larger than needed is shrunk by the publishing instruction.
  assert.equal(registryExtendsNeeded(100, registrySpace(256)), 0);
});
