const assert = require("node:assert/strict");
const { test } = require("node:test");
const { PublicKey } = require("@solana/web3.js");
const { BorshInstructionCoder } = require("@coral-xyz/anchor");
const fs = require("node:fs");
const path = require("node:path");
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
const { bn } = require("../dist/lib/utils");
const { findOpenEpoch } = require("../dist/commands/epoch");

const PROGRAM_ID = new PublicKey("chivcFQgxzwkpLvW41PV431HQ4dYpaW3povQH3AdpQt");
const hex = (bytes) => Buffer.from(bytes).toString("hex");

// Shared with `molpha-tests/tests/hashing.rs::ticket_hashes_match_golden_vectors`.
const FIXTURE = {
  value: new Uint8Array(32).fill(0x42),
  sourceId: new Uint8Array(32).fill(0x11),
  registryVersion: 4,
  signaturesRequired: 3,
  timestamp: 1_700_000_000_000, // unix milliseconds
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
        FIXTURE.timestamp,
      ),
    ),
    "1e5b3087286e8207743d03b04ec310a96b8c5250c45442fa6cfd77070c9940d8",
  );
  const messageHash = computeMessageHash(FIXTURE);
  assert.equal(
    hex(deriveTicketHash(messageHash, FIXTURE.aggSigS, FIXTURE.commitment)),
    "a39fe3fd44902947ffc92501bf1019847a1ceda7c7e29157c8a1951676bce8d2",
  );
  assert.equal(
    hex(
      deriveDispatchMessage(
        PROGRAM_ID,
        FIXTURE.sourceId,
        FIXTURE.signaturesRequired,
        FIXTURE.registryVersion,
        FIXTURE.timestamp,
      ),
    ),
    "5b77cc15f6940129344ceaf5cec37915d933ad0e4c378c2cac8ec334caaa7da7",
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

test("open epoch discovery survives closed accounts and index 64", async () => {
  const states = [
    {
      epoch: bn(3, "epoch"),
      start: bn(300, "start"),
      end: bn(400, "end"),
      finalized: 0,
    },
    {
      epoch: bn(64, "epoch"),
      start: bn(6400, "start"),
      end: bn(6500, "end"),
      finalized: 0,
    },
  ];
  const context = (now) => ({
    connection: { getSlot: async () => 1, getBlockTime: async () => now },
    program: {
      account: {
        epochState: { all: async () => states.map((account) => ({ account })) },
      },
    },
  });
  assert.equal(await findOpenEpoch(context(301), 0), 3);
  assert.equal(await findOpenEpoch(context(6401), 0), 64);
  await assert.rejects(findOpenEpoch(context(301), 4), /No open epoch/);
});

test(
  "epoch instructions encode u64 indices with Anchor BN",
  {
    skip: !fs.existsSync(
      path.resolve(__dirname, "../../target/idl/molpha.json"),
    ),
  },
  () => {
    const idl = require("../../target/idl/molpha.json");
    const coder = new BorshInstructionCoder(idl);
    const epoch = bn(64, "epoch");
    const next = bn(65, "epoch");
    const attestation = {
      payload: {
        value: Array(32).fill(0),
        sourceId: Array(32).fill(0),
        registryVersion: 0,
        signaturesRequired: 3,
        timestamp: bn(6401, "timestamp"),
      },
      signature: {
        aggSigS: Array(32).fill(0),
        commitment: Array(20).fill(0),
        signersBitmap: Array(32).fill(0),
      },
    };
    const commands = [
      ["finalize_epoch", { epoch }],
      ["credit_epoch_rewards", { epoch, slots: [] }],
      ["close_epoch", { epoch, openEpoch: next }],
      [
        "submit_ticket",
        {
          epoch,
          attestation,
          rosterSlots: [],
          coalitionKey: { x: Array(32).fill(0), y: Array(32).fill(0) },
        },
      ],
      ["sweep_treasury_revenue", { openEpoch: next }],
    ];
    for (const [name, args] of commands) {
      assert.ok(coder.encode(name, args).length > 8, `${name} should encode`);
    }
  },
);
