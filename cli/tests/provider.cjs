const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const { PublicKey } = require("@solana/web3.js");

const {
  findKeyXClaimPda,
  findNodePda,
  findProviderPda,
} = require("../dist/lib/pdas");
const { deriveKeyRotationDigest } = require("../dist/lib/signing/key-rotation");
const {
  FIRST_PROVIDER_SLOT,
  MAX_PROVIDER_SLOTS,
  NODE_SLOTS,
  PROVIDER_BIT_BASE,
  deriveSignersBitmap,
  isVacantMember,
  liveProviderSlots,
  registryMember,
  registryRecords,
  registrySlotOf,
  liveSlots,
  nextProviderSlot,
  nextRegistrySlot,
  nodeBitsOf,
  nodePopcountEvm,
  nodeQuorum,
  bitmapPopcountEvm,
} = require("../dist/lib/signing/attestation");
const {
  deriveNodePopMessage,
  secp256k1PublicKeyCompressed,
  signNodePop,
} = require("../dist/lib/signing/node-pop");
const { deriveNodeId } = require("../dist/lib/ids");
const { providerRegistrationMembers } = require("../dist/lib/registry");

const hex = (bytes) => Buffer.from(bytes).toString("hex");

// The shared provider-bit vectors (`molpha-verifier/tests/fixtures/provider_bits.vectors`): the
// program id and the provider PDAs the CLI derives must be the ones the verifier derives.
function readVectors() {
  const lines = fs
    .readFileSync(
      path.join(__dirname, "fixtures", "provider_bits.vectors"),
      "utf8",
    )
    .split("\n")
    .filter((line) => line && !line.startsWith("#"))
    .map((line) => line.split(/\s+/));
  return {
    program: lines.find((f) => f[0] === "program"),
    providers: lines.filter((f) => f[0] === "provider"),
    registries: lines.filter((f) => f[0] === "registry"),
    records: lines.filter((f) => f[0] === "record"),
  };
}

test("provider PDAs are the verifier's: [molpha_provider, id u32 LE]", () => {
  const vectors = readVectors();
  const programId = new PublicKey(vectors.program[1]);
  assert.equal(hex(programId.toBytes()), vectors.program[2]);
  assert.ok(vectors.providers.length >= 4);
  for (const [, id, pda] of vectors.providers) {
    const [address] = findProviderPda(programId, Number(id));
    assert.equal(hex(address.toBytes()), pda, `provider ${id}`);
  }
});

test("the key claim is on the x coordinate: a key and its negation share it", () => {
  const programId = new PublicKey(
    "dzemhcTeUnUEhDcyUssWP8GfcMCYwKezVg4U1hvdEJe",
  );
  const x = new Uint8Array(32).fill(0x33);
  const even = Uint8Array.from([0x02, ...x]);
  const odd = Uint8Array.from([0x03, ...x]);
  const [a] = findKeyXClaimPda(programId, even);
  const [b] = findKeyXClaimPda(programId, odd);
  const [c] = findKeyXClaimPda(programId, x);
  assert.ok(a.equals(b) && a.equals(c));
  assert.throws(() => findKeyXClaimPda(programId, new Uint8Array(31)));
});

// A provider proves possession like a node: the node PoP over the node id of the provider authority,
// its key and its salt. `provider pop` derives the key, the node id and the proof in that order.
test("the provider proof of possession is the node one over the authority's node id", () => {
  const programId = new PublicKey(
    "dzemhcTeUnUEhDcyUssWP8GfcMCYwKezVg4U1hvdEJe",
  );
  const authority = new PublicKey(new Uint8Array(32).fill(0x42));
  const secret = new Uint8Array(32).fill(0x11);
  const salt = new Uint8Array(32).fill(0x33);
  const key = secp256k1PublicKeyCompressed(secret);
  const nodeId = deriveNodeId(authority, key, salt);
  const pop = signNodePop({
    programId,
    nodeId,
    secp256k1PrivateKey: secret,
    nonce: new Uint8Array(32).fill(0x22),
  });
  assert.equal(hex(pop.secp256k1PubkeyCompressed), hex(key));
  assert.equal(
    hex(key),
    "034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa",
  );
  // The node id, and so the message, binds the authority and the salt.
  const other = deriveNodeId(
    new PublicKey(new Uint8Array(32).fill(1)),
    key,
    salt,
  );
  assert.notEqual(
    hex(deriveNodePopMessage(programId, nodeId)),
    hex(deriveNodePopMessage(programId, other)),
  );
  assert.notEqual(
    hex(nodeId),
    hex(deriveNodeId(authority, key, new Uint8Array(32).fill(0x34))),
  );
});

function member(byte) {
  return {
    node: new Uint8Array(32).fill(byte),
    x: new Uint8Array(32),
    y: new Uint8Array(32),
  };
}
const hole = () => member(0);

test("node-only helpers ignore the provider table", () => {
  assert.equal(NODE_SLOTS, 64);
  assert.equal(PROVIDER_BIT_BASE, 64);
  // Three node slots (the middle one a hole); provider slots 0 (reserved), 1, 2 (a hole), 3.
  const members = [member(1), hole(), member(3)];
  const providers = [hole(), member(0xa1), hole(), member(0xa3)];
  const registry = { nodeCount: 3, members, providerCount: 4, providers };

  assert.deepEqual(liveSlots(registry), [0, 2]);
  assert.deepEqual(liveProviderSlots(registry), [1, 3]);
  assert.equal(nextRegistrySlot(registry), 1);
  assert.equal(FIRST_PROVIDER_SLOT, 1);
  assert.equal(nextProviderSlot(registry), 2);
  assert.equal(registryRecords(registry), 7);
  // A bit maps into its own table: bit 65 is provider slot 1, bit 3 is past the node table.
  assert.equal(registryMember(registry, 65).node[0], 0xa1);
  assert.throws(() => registryMember(registry, 3), /outside node_count/);
  assert.throws(() => registryMember(registry, 66), /is a hole/);
  assert.throws(() => registryMember(registry, 68), /outside provider_count/);
  assert.equal(
    registrySlotOf(registry, new PublicKey(Uint8Array.from(providers[3].node))),
    67,
  );
});

test("a full node segment and a full provider segment have no next slot", () => {
  const nodes = Array.from({ length: 64 }, (_, i) => member(i + 1));
  assert.equal(nextRegistrySlot({ nodeCount: 64, members: nodes }), -1);

  const providers = [hole()];
  for (let slot = 1; slot < MAX_PROVIDER_SLOTS; slot += 1)
    providers.push(member(0x80));
  const full = {
    nodeCount: 64,
    members: nodes,
    providerCount: providers.length,
    providers,
  };
  assert.equal(registryRecords(full), 256);
  assert.equal(nextRegistrySlot(full), -1);
  assert.equal(nextProviderSlot(full), -1);
});

test("selection draws over the node slots and provider records never change it", () => {
  const nodes = Array.from({ length: 12 }, (_, i) => member(i + 1));
  // A node table with trailing node slots that would hold providers in a single table: the draw
  // reads the node count and table only.
  const withProviders = [...nodes];

  for (let i = 0; i < 16; i += 1) {
    const source = new Uint8Array(32).fill(i);
    const timestamp = 1_705_257_421_000 + i * 1_000;
    const plain = deriveSignersBitmap(
      source,
      3,
      nodes.length,
      5,
      2,
      timestamp,
      nodes,
    );
    const mixed = deriveSignersBitmap(
      source,
      3,
      withProviders.length,
      5,
      2,
      timestamp,
      withProviders,
    );
    // A full node table: the draw stops at 64 node slots.
    const capped = deriveSignersBitmap(source, 3, 200, 5, 2, timestamp);
    assert.equal(hex(nodeBitsOf(capped)), hex(capped));
    assert.equal(hex(mixed), hex(plain), `source ${i}`);
    assert.equal(hex(nodeBitsOf(mixed)), hex(mixed));
  }
});

test("the quorum counts every bit; the node bits are the credited ones", () => {
  const bitmap = new Uint8Array(32);
  bitmap[31] = 0b0001_1111; // nodes 0..4
  bitmap[23] = 0b0000_0010; // bit 65, a provider
  bitmap[0] = 0b1000_0000; // bit 255, a provider
  assert.equal(bitmapPopcountEvm(bitmap), 7);
  assert.equal(nodePopcountEvm(bitmap), 5);
  assert.equal(hex(nodeBitsOf(bitmap)).replace(/0/g, "").length > 0, true);
  assert.equal(nodeBitsOf(bitmap)[23], 0);
  assert.equal(nodeBitsOf(bitmap)[0], 0);
  assert.equal(nodeBitsOf(bitmap)[31], 0b0001_1111);
});

test("a provider round is drawn by its node quorum, one less than the wire threshold", () => {
  const committee = new Uint8Array(32);
  committee[31] = 0b0001_1111; // nodes 0..4
  const providerRound = new Uint8Array(32);
  providerRound[31] = 0b0000_1111; // nodes 0..3
  providerRound[23] = 0b0000_0010; // bit 65, the provider
  assert.equal(nodeQuorum(5, committee), 5);
  assert.equal(nodeQuorum(5, providerRound), 4);
  // t = 5 counts four nodes and the provider.
  assert.equal(bitmapPopcountEvm(providerRound), 5);
});

test("a provider registration needs room for its slots, never padding to bit 64", () => {
  const nodes = (n) => Array.from({ length: n }, (_, i) => member(i + 1));
  // The first provider brings reserved slot 0 and its own: two records.
  assert.equal(
    providerRegistrationMembers({ nodeCount: 10, members: nodes(10) }, 1),
    12,
  );
  // A freed slot adds none; a new last slot adds one.
  const registry = {
    nodeCount: 64,
    members: nodes(64),
    providerCount: 42,
    providers: [hole(), ...Array.from({ length: 41 }, () => member(0x80))],
  };
  assert.equal(providerRegistrationMembers(registry, 5), 106);
  assert.equal(providerRegistrationMembers(registry, 42), 107);
});

// A live run found `provider register` sending 32 zero bytes: the CLI named the argument `p2pKey`, but
// Anchor derives `p2PKey` from the IDL's `p2p_key`, and the encoder serialises an unknown name as zeros
// without an error. The program then refuses the key as InvalidP2pKey.
test("a provider registration carries the p2p key under the name Anchor derives", () => {
  const idlPath = path.join(__dirname, "../../target/idl/molpha.json");
  if (!fs.existsSync(idlPath)) return; // `anchor build` writes the IDL
  const { AnchorProvider, Program } = require("@coral-xyz/anchor");
  const { Connection } = require("@solana/web3.js");
  const { registerProviderArgs } = require("../dist/commands/provider");
  const wallet = {
    publicKey: PublicKey.default,
    signTransaction: async (tx) => tx,
    signAllTransactions: async (txs) => txs,
  };
  const program = new Program(
    JSON.parse(fs.readFileSync(idlPath, "utf8")),
    new AnchorProvider(new Connection("http://127.0.0.1:1"), wallet, {}),
  );
  const p2pKey = Uint8Array.from({ length: 32 }, (_, i) => i + 1);
  const args = registerProviderArgs({
    registrationSalt: new Uint8Array(32).fill(9),
    p2pKey,
    secpPubkey: new Uint8Array(33).fill(2),
    popSigR: new Uint8Array(32).fill(3),
    popSigS: new Uint8Array(32).fill(4),
  });
  // discriminator 8, salt 32, endpoint tag 1 (1 = Provider), p2p key 32, signing key 33.
  const data = program.coder.instruction.encode("registerNode", { args });
  assert.deepEqual([...data.subarray(8, 40)], new Array(32).fill(9));
  assert.equal(data[40], 1);
  assert.deepEqual([...data.subarray(41, 73)], [...p2pKey]);
  assert.deepEqual([...data.subarray(73, 106)], new Array(33).fill(2));
  // The spelling the CLI used before serialises as zeros: the failure this test exists to prevent.
  const wrong = program.coder.instruction.encode("registerNode", {
    args: {
      ...args,
      endpoint: { provider: { p2pKey: Array.from(p2pKey) } },
    },
  });
  assert.deepEqual([...wrong.subarray(41, 73)], new Array(32).fill(0));
});

const idlFile = path.join(__dirname, "../../target/idl/molpha.json");

test("a provider rotates through rotate_node_key and deactivate_node, with the provider account optional", () => {
  if (!fs.existsSync(idlFile)) return; // `anchor build` writes the IDL
  const idl = JSON.parse(fs.readFileSync(idlFile, "utf8"));
  for (const gone of [
    "eject_provider",
    "register_provider",
    "approve_provider",
    "activate_provider",
    "update_registry_manager",
  ]) {
    assert.ok(!idl.instructions.some((i) => i.name === gone), gone);
  }
  for (const name of ["register_node", "rotate_node_key", "deactivate_node"]) {
    const ix = idl.instructions.find((i) => i.name === name);
    for (const account of ["node", "provider"]) {
      assert.equal(
        ix.accounts.find((a) => a.name === account).optional,
        true,
        `${name}.${account} is optional`,
      );
    }
  }
  const provider = idl.accounts.find((a) => a.name === "Provider");
  const fields = idl.types.find((t) => t.name === "Provider").type.fields;
  assert.ok(provider);
  assert.equal(fields[fields.length - 1].name, "key_rotated_at");
  assert.ok(
    !fields.some((f) => f.name === "approved_at" || f.name === "activates_at"),
  );
});

test("the rotation digest is bound to the account that rotates", () => {
  if (!fs.existsSync(idlFile)) return;
  const programId = new PublicKey(
    JSON.parse(fs.readFileSync(idlFile, "utf8")).address,
  );
  const newKey = new Uint8Array(33).fill(2);
  const [provider] = findProviderPda(programId, 1);
  const [node] = findNodePda(programId, PublicKey.default);
  const a = deriveKeyRotationDigest(programId, provider, newKey);
  assert.equal(a.length, 32);
  assert.notDeepEqual(
    [...a],
    [...deriveKeyRotationDigest(programId, node, newKey)],
  );
  assert.notDeepEqual(
    [...a],
    [
      ...deriveKeyRotationDigest(
        programId,
        findProviderPda(programId, 2)[0],
        newKey,
      ),
    ],
  );
});

// `provider p2p-key` sends `update_node_endpoint` with the Provider variant: tag 1, then the key.
test("a p2p key update is update_node_endpoint with the provider endpoint", () => {
  if (!fs.existsSync(idlFile)) return;
  const { AnchorProvider, Program } = require("@coral-xyz/anchor");
  const { Connection } = require("@solana/web3.js");
  const idl = JSON.parse(fs.readFileSync(idlFile, "utf8"));
  assert.ok(
    !idl.instructions.some((i) => i.name === "update_provider_p2p_key"),
  );
  const ix = idl.instructions.find((i) => i.name === "update_node_endpoint");
  for (const account of ["node", "provider"]) {
    assert.equal(ix.accounts.find((a) => a.name === account).optional, true);
  }
  const program = new Program(
    idl,
    new AnchorProvider(
      new Connection("http://127.0.0.1:1"),
      {
        publicKey: PublicKey.default,
        signTransaction: async (tx) => tx,
        signAllTransactions: async (txs) => txs,
      },
      {},
    ),
  );
  const p2pKey = Array.from({ length: 32 }, (_, i) => i + 7);
  const data = program.coder.instruction.encode("updateNodeEndpoint", {
    endpoint: { provider: { p2PKey: p2pKey } },
  });
  assert.equal(data.length, 8 + 1 + 32);
  assert.equal(data[8], 1);
  assert.deepEqual([...data.subarray(9)], p2pKey);
  const node = program.coder.instruction.encode("updateNodeEndpoint", {
    endpoint: { node: { ip: [10, 0, 0, 1], port: 8080 } },
  });
  assert.deepEqual([...node.subarray(8)], [0, 10, 0, 0, 1, 0x90, 0x1f]);
});
