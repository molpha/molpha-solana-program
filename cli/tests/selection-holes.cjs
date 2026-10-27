const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { PublicKey } = require("@solana/web3.js");

const {
  deriveSignersBitmap,
  isVacantMember,
  liveSlots,
  nextRegistrySlot,
  registryNode,
  registrySlotOf,
} = require("../dist/lib/signing/attestation");

const hex = (bytes) => Buffer.from(bytes).toString("hex");

/** A registry of `slots` slots whose live ones are the set bits of `liveHex`. */
function registryFromLive(liveHex) {
  const live = Buffer.from(liveHex, "hex");
  const bitSet = (slot) => (live[31 - (slot >> 3)] >> (slot & 7)) & 1;
  let last = 0;
  for (let slot = 0; slot < 256; slot += 1) if (bitSet(slot)) last = slot;
  const members = [];
  for (let slot = 0; slot <= last; slot += 1) {
    members.push({
      node: new Uint8Array(32).fill(bitSet(slot) ? (slot % 255) + 1 : 0),
      x: new Uint8Array(32),
      y: new Uint8Array(32),
    });
  }
  return { nodeCount: members.length, members };
}

// The same vectors the verifier crate pins (`molpha-verifier/tests/fixtures/selection_holes.vectors`).
test("selection over a registry with holes matches the shared vectors", () => {
  const lines = fs
    .readFileSync(
      path.join(__dirname, "fixtures", "selection_holes.vectors"),
      "utf8",
    )
    .trim()
    .split("\n");
  assert.equal(lines.length, 48);
  for (const line of lines) {
    const [liveHex, source, version, timestamp, required, buffer, expected] =
      line.split(" ");
    const registry = registryFromLive(liveHex);
    const got = deriveSignersBitmap(
      Buffer.from(source, "hex"),
      Number(version),
      registry.nodeCount,
      Number(required),
      Number(buffer),
      Number(timestamp),
      registry.members,
    );
    assert.equal(hex(got), expected, line);
  }
});

test("without holes the selection is the plain draw over the slots", () => {
  const registry = registryFromLive("00".repeat(30) + "0fff");
  const source = new Uint8Array(32).fill(7);
  const withMembers = deriveSignersBitmap(
    source,
    3,
    12,
    5,
    2,
    1_705_257_421_000,
    registry.members,
  );
  const without = deriveSignersBitmap(source, 3, 12, 5, 2, 1_705_257_421_000);
  assert.equal(hex(withMembers), hex(without));
});

test("slot helpers skip holes and find the lowest free slot", () => {
  const registry = registryFromLive("00".repeat(30) + "00b5"); // slots 0, 2, 4, 5, 7
  assert.deepEqual(liveSlots(registry), [0, 2, 4, 5, 7]);
  assert.equal(isVacantMember(registry.members[1]), true);
  assert.equal(nextRegistrySlot(registry), 1);
  assert.throws(() => registryNode(registry, 1), /is a hole/);

  const node = new PublicKey(Uint8Array.from(registry.members[4].node));
  assert.equal(registrySlotOf(registry, node), 4);
  assert.equal(registrySlotOf(registry, new PublicKey(new Uint8Array(32))), -1);

  const full = registryFromLive("00".repeat(30) + "000f");
  assert.equal(nextRegistrySlot(full), 4);
});
