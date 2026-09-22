const assert = require("node:assert/strict");
const { test } = require("node:test");
const { PublicKey } = require("@solana/web3.js");
const { bitmapFromBits, creditedBitsFromBitmap } = require("../dist/lib/agg-sig");
const {
  deriveSignersBitmap,
  settlementRemainingAccounts,
} = require("../dist/lib/signing/attestation");

test("settlement accounts put signers before selected misses and make every node writable", () => {
  const registry = {
    version: 7,
    nodeCount: 8,
    redundancyBuffer: 2,
    nodes: Array.from({ length: 8 }, (_, index) =>
      Array.from(new PublicKey(Uint8Array.from({ length: 32 }, () => index + 1)).toBytes()),
    ),
  };
  const payload = {
    sourceId: Array(32).fill(0x11),
    registryVersion: registry.version,
    signaturesRequired: 3,
    canonicalTimestamp: 1_700_000_000,
  };
  const selection = deriveSignersBitmap(
    Uint8Array.from(payload.sourceId),
    payload.registryVersion,
    registry.nodeCount,
    payload.signaturesRequired,
    registry.redundancyBuffer,
    payload.canonicalTimestamp,
  );
  const selectedBits = creditedBitsFromBitmap(selection);
  const signerBits = selectedBits.slice(0, payload.signaturesRequired);
  const accounts = settlementRemainingAccounts(
    registry,
    payload,
    bitmapFromBits(signerBits),
  );

  const expectedBits = [
    ...signerBits,
    ...selectedBits.filter((bit) => !signerBits.includes(bit)),
  ];
  assert.deepEqual(
    accounts.map((account) => account.pubkey.toBase58()),
    expectedBits.map((bit) => new PublicKey(registry.nodes[bit]).toBase58()),
  );
  assert.ok(accounts.every((account) => account.isWritable && !account.isSigner));
});

test("settlement account derivation rejects a signer outside selection", () => {
  const registry = {
    version: 1,
    nodeCount: 8,
    redundancyBuffer: 2,
    nodes: Array.from({ length: 8 }, (_, index) => Array(32).fill(index + 1)),
  };
  const payload = {
    sourceId: Array(32).fill(0x22),
    registryVersion: 1,
    signaturesRequired: 3,
    canonicalTimestamp: 1_700_000_001,
  };
  const selection = deriveSignersBitmap(
    Uint8Array.from(payload.sourceId),
    payload.registryVersion,
    registry.nodeCount,
    payload.signaturesRequired,
    registry.redundancyBuffer,
    payload.canonicalTimestamp,
  );
  const selected = new Set(creditedBitsFromBitmap(selection));
  const outsider = Array.from({ length: registry.nodeCount }, (_, bit) => bit).find(
    (bit) => !selected.has(bit),
  );
  assert.notEqual(outsider, undefined);
  assert.throws(
    () => settlementRemainingAccounts(registry, payload, bitmapFromBits([outsider])),
    /outside the derived selection/,
  );
  assert.throws(
    () =>
      settlementRemainingAccounts(
        registry,
        payload,
        bitmapFromBits([registry.nodeCount]),
      ),
    /outside the registry/,
  );
});
