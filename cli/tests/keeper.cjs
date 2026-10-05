const assert = require("node:assert/strict");
const { test } = require("node:test");
const { planKeeper, KEEPER_DEFAULTS } = require("../dist/lib/keeper");
const { RpcPool, isTransportError } = require("../dist/commands/keeper");

const LEN = 86_400;
const GRACE = 21_600;

function epoch(index, over = {}) {
  const start = 1_000_000 + index * LEN;
  return {
    epoch: index,
    start,
    end: start + LEN,
    ticketGrace: GRACE,
    finalized: false,
    weightedCount: 0,
    creditedCount: 0,
    creditSlots: [],
    ...over,
  };
}

const config = (over = {}) => ({
  ...KEEPER_DEFAULTS,
  lastSweepAt: null,
  ...over,
});

const kinds = (plan) => plan.actions.map((a) => `${a.kind}:${a.epoch}`);

test("a healthy lifecycle only sweeps", () => {
  // Epoch 0 open, epoch 1 is the one ahead.
  const now = epoch(0).start + 3_600;
  const plan = planKeeper([epoch(0), epoch(1)], now, config());
  assert.deepEqual(kinds(plan), ["sweep:0"]);
  assert.deepEqual(plan.health, []);
});

test("sweeps are throttled", () => {
  const now = epoch(0).start + 3_600;
  const plan = planKeeper(
    [epoch(0), epoch(1)],
    now,
    config({ lastSweepAt: now - 60 }),
  );
  assert.deepEqual(kinds(plan), []);
});

test("an epoch past its ticket deadline is finalized once its successor exists", () => {
  const now = epoch(0).end + GRACE + 10;
  const plan = planKeeper([epoch(0), epoch(1)], now, config());
  assert.deepEqual(
    kinds(plan).filter((k) => k.startsWith("finalize")),
    ["finalize:0"],
  );
  // Epoch 1 is open; finalize is fresh, not overdue.
  assert.deepEqual(plan.health, []);
});

test("a long outage is caught up in order, each finalize enabling the next", () => {
  // Nobody ran the keeper: epochs 0 and 1 are both past their deadlines, 2 does not exist.
  const now = epoch(1).end + GRACE + 10;
  const plan = planKeeper([epoch(0), epoch(1)], now, config());
  assert.deepEqual(
    kinds(plan).filter((k) => k.startsWith("finalize")),
    ["finalize:0", "finalize:1"],
  );
  const codes = plan.health.map((h) => h.code).sort();
  // Epoch 0 is a day late; epoch 1 only just passed its deadline, inside the tolerance.
  assert.deepEqual(codes, ["finalize-overdue", "no-open-epoch"]);
  assert.ok(plan.health.every((h) => h.level === "error"));
});

test("finalizing is not planned while the next epoch is missing or the successor exists", () => {
  const now = epoch(0).end + GRACE + 10;
  assert.deepEqual(
    kinds(planKeeper([epoch(0)], now, config())).filter((k) =>
      k.startsWith("finalize"),
    ),
    [],
  );
  assert.deepEqual(
    kinds(planKeeper([epoch(0), epoch(1), epoch(2)], now, config())).filter(
      (k) => k.startsWith("finalize"),
    ),
    [],
  );
});

test("a finalize overdue by less than the tolerance is not an alert", () => {
  const now = epoch(0).end + GRACE + KEEPER_DEFAULTS.finalizeOverdueSeconds - 1;
  const plan = planKeeper([epoch(0), epoch(1)], now, config());
  assert.deepEqual(plan.health, []);
  const later = planKeeper([epoch(0), epoch(1)], now + 2, config());
  assert.deepEqual(
    later.health.map((h) => h.code),
    ["finalize-overdue"],
  );
});

test("the runway warning fires when the newest epoch is about to end", () => {
  const now = epoch(1).end - 600;
  const plan = planKeeper([epoch(1)], now, config());
  assert.deepEqual(
    plan.health.map((h) => h.code),
    ["epoch-runway-low"],
  );
  assert.equal(plan.health[0].level, "warn");
});

test("finalized epochs are credited, then closed after their evidence window", () => {
  const done = epoch(0, {
    finalized: true,
    weightedCount: 3,
    creditedCount: 0,
    creditSlots: [0, 1, 2],
  });
  const open = epoch(2);
  const now = open.start + 10;
  let plan = planKeeper(
    [done, epoch(1, { finalized: true }), open, epoch(3)],
    now,
    config({ lastSweepAt: now }),
  );
  assert.ok(kinds(plan).includes("credit:0"));
  assert.ok(
    !kinds(plan).includes("close:0"),
    "an uncredited epoch cannot be closed",
  );

  const credited = { ...done, creditedCount: 3, creditSlots: [] };
  // Evidence window: one epoch length after the ticket deadline.
  const early = done.end + GRACE + LEN - 1;
  plan = planKeeper(
    [credited, epoch(1), open, epoch(3)],
    early,
    config({ lastSweepAt: early }),
  );
  assert.ok(!kinds(plan).includes("close:0"));
  const late = done.end + GRACE + LEN;
  const lateOpen = { ...epoch(2), start: late - 100, end: late + 100 };
  plan = planKeeper(
    [credited, epoch(1), lateOpen, epoch(3)],
    late,
    config({ lastSweepAt: late }),
  );
  const close = plan.actions.find((a) => a.kind === "close" && a.epoch === 0);
  assert.ok(close, "closable after the evidence window");
  assert.equal(close.openEpoch, 2);
});

test("transport errors are told apart from program errors", () => {
  for (const message of [
    "fetch failed",
    "429 Too Many Requests",
    "503 Service Unavailable",
    "ECONNREFUSED 127.0.0.1:8899",
  ]) {
    assert.ok(isTransportError(new Error(message)), message);
  }
  for (const message of [
    "custom program error: 0x1778",
    "Error Code: EpochNotFinalizable",
  ]) {
    assert.ok(!isTransportError(new Error(message)), message);
  }
});

test("the RPC pool falls back on transport errors and sticks to the endpoint that works", async () => {
  const ctx = (name) => ({
    options: { rpcUrl: `http://${name}.example` },
    name,
  });
  const pool = new RpcPool([ctx("a"), ctx("b")]);
  const calls = [];
  const fn = async (c) => {
    calls.push(c.name);
    if (c.name === "a") throw new Error("fetch failed");
    return c.name;
  };
  assert.equal(await pool.run(fn), "b");
  assert.deepEqual(calls, ["a", "b"]);
  calls.length = 0;
  assert.equal(await pool.run(fn), "b");
  assert.deepEqual(
    calls,
    ["b"],
    "the working endpoint is tried first next time",
  );

  // A program error is the transaction's own: never retried elsewhere.
  const failing = new RpcPool([ctx("a"), ctx("b")]);
  const seen = [];
  await assert.rejects(
    failing.run(async (c) => {
      seen.push(c.name);
      throw new Error("custom program error: 0x1778");
    }),
    /0x1778/,
  );
  assert.deepEqual(seen, ["a"]);
});
