const assert = require("node:assert/strict");
const { test } = require("node:test");
const { PublicKey } = require("@solana/web3.js");
const {
  deriveDispatchMessage,
  deriveTicketHash,
  deriveTicketId,
  epochWeight,
  isTicket,
  rosterSlots,
} = require("../dist/lib/ticket");
const { computeMessageHash } = require("../dist/lib/signing/attestation");
const { inactivityEvidenceHash } = require("../dist/lib/slash");
const { findEpochPda } = require("../dist/lib/pdas");

const PROGRAM_ID = new PublicKey("MoLFnEbuMS5gWnXNfUMLAYSqRM3eQZKWRzjeMQfqbT3");
const hex = (bytes) => Buffer.from(bytes).toString("hex");

// Shared with `molpha-tests/tests/hashing.rs::ticket_hashes_match_golden_vectors`.
const FIXTURE = {
  value: new Uint8Array(32).fill(0x42),
  sourceId: new Uint8Array(32).fill(0x11),
  registryVersion: 4,
  signaturesRequired: 3,
  canonicalTimestamp: 1_700_000_000,
  aggSigS: new Uint8Array(32).fill(0x66),
  commitment: new Uint8Array(20).fill(0x77),
  signersBitmap: new Uint8Array(32).fill(0x07),
};

test("ticket id, ticket hash and dispatch message match the program's golden vectors", () => {
  assert.equal(
    hex(
      deriveTicketId(
        FIXTURE.sourceId,
        FIXTURE.signaturesRequired,
        FIXTURE.canonicalTimestamp,
      ),
    ),
    "859d832c8a492a6d0b04abdd1c4ea66c8473ed4879c644e8b9f4a28629edf196",
  );
  const messageHash = computeMessageHash(FIXTURE);
  assert.equal(
    hex(deriveTicketHash(messageHash, FIXTURE.aggSigS, FIXTURE.commitment)),
    "e99e3d17eba0eefa7bd0cac0760f3092d28b467714b3f8f3bbd1643133bcf2e5",
  );
  assert.equal(
    hex(
      deriveDispatchMessage(
        PROGRAM_ID,
        FIXTURE.sourceId,
        FIXTURE.signaturesRequired,
        FIXTURE.registryVersion,
        FIXTURE.canonicalTimestamp,
      ),
    ),
    "73b2adeadd7b47a21b53811238619e453501035fa2c178c293dc02ed1dc6a6e9",
  );
});

test("the lottery compares the big-endian u64 prefix to tau", () => {
  const hash = new Uint8Array(32);
  hash[7] = 5;
  assert.equal(isTicket(hash, 6n), true);
  assert.equal(isTicket(hash, 5n), false);
  assert.equal(isTicket(hash, 0n), false);
});

test("roster hints reuse known slots and append new nodes in selection order", () => {
  const key = (n) => new PublicKey(new Uint8Array(32).fill(n)).toBytes();
  const registry = { nodeCount: 4, nodes: [key(1), key(2), key(3), key(4)] };
  const epoch = { rosterLen: 2, roster: [key(3), key(1)] };
  // Bits 0, 2 and 3 selected (big-endian bitmap, bit 0 in the last byte).
  const selection = new Uint8Array(32);
  selection[31] = 0b1101;
  assert.deepEqual(rosterSlots(epoch, registry, selection), [1, 0, 2]);
});

test("payout weight applies the relative availability floor only above the sample minimum", () => {
  const epoch = {
    rosterLen: 2,
    signed: [1, 4],
    selected: [4, 4],
    sumSigned: 5,
    sumSelected: 8,
    minSamples: 4,
    minAvailabilityBps: 8000,
    credited: new Array(32).fill(0),
  };
  // Network rate 5/8; floor 0.8 x 0.625 = 0.5; slot 0 sits at 0.25.
  assert.equal(epochWeight(epoch, 0), 0n);
  assert.equal(epochWeight(epoch, 1), 4n);
  assert.equal(epochWeight({ ...epoch, minSamples: 5 }, 0), 1n);
});

test("inactivity cases hash the epoch and roster slot", () => {
  assert.notEqual(
    hex(inactivityEvidenceHash(4, 1)),
    hex(inactivityEvidenceHash(4, 2)),
  );
  assert.notEqual(
    hex(inactivityEvidenceHash(4, 1)),
    hex(inactivityEvidenceHash(5, 1)),
  );
});

test("epoch PDAs use a little-endian u64 index", () => {
  const [pda] = findEpochPda(PROGRAM_ID, 7);
  const [expected] = PublicKey.findProgramAddressSync(
    [Buffer.from("molpha_epoch"), Buffer.from([7, 0, 0, 0, 0, 0, 0, 0])],
    PROGRAM_ID,
  );
  assert.ok(pda.equals(expected));
});
