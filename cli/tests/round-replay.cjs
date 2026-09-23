const assert = require("node:assert/strict");
const { test } = require("node:test");
const { Keypair, PublicKey } = require("@solana/web3.js");
const {
  RoundReplayKind,
  ROUND_SEED,
  deriveRoundId,
  findRoundReplayPda,
  findSubscriptionRoundPda,
  findX402RoundPda,
} = require("../dist/lib/pdas");
const { roundIdFromOptions } = require("../dist/commands/round");

const PROGRAM_ID = new PublicKey("MoLFnEbuMS5gWnXNfUMLAYSqRM3eQZKWRzjeMQfqbT3");
const SOURCE_ID = new Uint8Array(32).fill(0x11);
const INITIATOR = new Uint8Array(32).fill(0x22);

// Shared with `molpha-tests/tests/round_replay.rs::round_id_matches_the_shared_vector`.
const X402_VECTOR =
  "ccdd6789c23b7294903c844f1dc2645fee57072e356dfc4c2c9f575cd8ee0db6";
const SUBSCRIPTION_VECTOR =
  "736d334ad6b5c414d1d895130a50a3b089da716b5ebe44b2fb48d1931200fd65";

test("round id matches the program's golden vectors", () => {
  const hex = (bytes) => Buffer.from(bytes).toString("hex");
  assert.equal(
    hex(
      deriveRoundId(
        RoundReplayKind.X402,
        SOURCE_ID,
        5,
        1_700_000_000,
        INITIATOR,
      ),
    ),
    X402_VECTOR,
  );
  assert.equal(
    hex(
      deriveRoundId(
        RoundReplayKind.Subscription,
        SOURCE_ID,
        5,
        1_700_000_000,
        INITIATOR,
      ),
    ),
    SUBSCRIPTION_VECTOR,
  );
});

test("replay markers are [molpha_round, gateway, roundId] and scoped per path", () => {
  const gateway = Keypair.generate().publicKey;
  const consumer = new PublicKey(INITIATOR);
  const [sub] = findSubscriptionRoundPda(
    PROGRAM_ID,
    consumer,
    gateway,
    SOURCE_ID,
    5,
    1_700_000_000,
  );
  const [x402] = findX402RoundPda(
    PROGRAM_ID,
    INITIATOR,
    gateway,
    SOURCE_ID,
    5,
    1_700_000_000,
  );
  assert.notEqual(sub.toBase58(), x402.toBase58());

  const [expected] = PublicKey.findProgramAddressSync(
    [ROUND_SEED, gateway.toBuffer(), Buffer.from(X402_VECTOR, "hex")],
    PROGRAM_ID,
  );
  assert.equal(x402.toBase58(), expected.toBase58());
  assert.equal(
    findRoundReplayPda(
      PROGRAM_ID,
      gateway,
      Buffer.from(X402_VECTOR, "hex"),
    )[0].toBase58(),
    expected.toBase58(),
  );
});

test("round id derivation rejects malformed inputs", () => {
  assert.throws(
    () =>
      deriveRoundId(RoundReplayKind.X402, new Uint8Array(31), 5, 1, INITIATOR),
    /source id/,
  );
  assert.throws(
    () =>
      deriveRoundId(RoundReplayKind.X402, SOURCE_ID, 5, 1, new Uint8Array(31)),
    /initiator/,
  );
  assert.throws(
    () => deriveRoundId(RoundReplayKind.X402, SOURCE_ID, 256, 1, INITIATOR),
    /u8/,
  );
});

test("close-replay derives the round id from its options", () => {
  const options = {
    kind: "x402",
    initiator: `0x${Buffer.from(INITIATOR).toString("hex")}`,
    sourceId: `0x${Buffer.from(SOURCE_ID).toString("hex")}`,
    signaturesRequired: "5",
    canonicalTimestamp: "1700000000",
  };
  assert.equal(
    Buffer.from(roundIdFromOptions(options)).toString("hex"),
    X402_VECTOR,
  );
  assert.equal(
    Buffer.from(
      roundIdFromOptions({
        ...options,
        kind: "subscription",
        initiator: new PublicKey(INITIATOR).toBase58(),
      }),
    ).toString("hex"),
    SUBSCRIPTION_VECTOR,
  );
  assert.throws(
    () => roundIdFromOptions({ ...options, kind: "other" }),
    /--kind/,
  );
  assert.throws(() => roundIdFromOptions({ kind: "x402" }), /missing/);
});
