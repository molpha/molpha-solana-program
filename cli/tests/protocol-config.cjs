const assert = require("node:assert/strict");
const { test } = require("node:test");
const { BN } = require("@coral-xyz/anchor");
const { buildUpdateConfigArgs } = require("../dist/commands/update-config");
const { validateProtocolConfig } = require("../dist/lib/protocol-config");
const { Command } = require("commander");
const { registerInitCommand } = require("../dist/commands/init");
const {
  registerUpdateConfigCommand,
} = require("../dist/commands/update-config");

function config() {
  return {
    withdrawalCooldownSlots: new BN(3),
    minimumNodeDeposit: new BN(100),
    rewardPerSignature: new BN(10),
    maxSettlementDelaySeconds: new BN(256),
    challengerBountyBps: 500,
    x402RoundBase: new BN(1000),
    minSigners: 3,
    epochLenSeconds: new BN(86400),
    ticketGraceSeconds: new BN(21600),
    targetTicketsPerEpoch: 2000,
    minAvailabilityBps: 8000,
    minAvailabilitySamples: 50,
    protocolFeeBps: 500,
  };
}

test("partial updates preserve current rates and flags override JSON as numbers", () => {
  const current = config();
  assert.deepEqual(buildUpdateConfigArgs({}, {}, current), current);
  const args = buildUpdateConfigArgs(
    { protocolFeeBps: "800", minSigners: "4" },
    { protocolFeeBps: 900, minAvailabilityBps: 7000 },
    current,
  );
  assert.equal(args.protocolFeeBps, 800);
  assert.equal(args.minAvailabilityBps, 7000);
  assert.equal(args.minSigners, 4);
  assert.equal(args.epochLenSeconds.toString(), "86400");
});

test("init and update commands expose the epoch knobs and no liveliness or gateway fee knobs", () => {
  for (const register of [registerInitCommand, registerUpdateConfigCommand]) {
    const program = new Command();
    register(program);
    const flags = program.commands[0].options.map((option) => option.long);
    for (const flag of [
      "--epoch-len-seconds",
      "--ticket-grace-seconds",
      "--target-tickets-per-epoch",
      "--min-availability-bps",
      "--min-availability-samples",
      "--protocol-fee-bps",
    ]) {
      assert.ok(flags.includes(flag), flag);
    }
    assert.ok(!flags.some((flag) => flag.startsWith("--liveliness")));
    assert.ok(!flags.includes("--gateway-fee-bps"));
  }
});

test("invalid integer knobs are rejected before encoding", () => {
  for (const field of ["protocolFeeBps", "minAvailabilityBps"]) {
    for (const value of ["-1", "10001", "1.5", "NaN", "Infinity"]) {
      assert.throws(
        () => buildUpdateConfigArgs({ [field]: value }, {}, config()),
        new RegExp(field),
      );
    }
  }
  for (const [field, value] of [
    ["minSigners", "256"],
    ["minSigners", "2"],
    ["challengerBountyBps", "10001"],
    ["targetTicketsPerEpoch", "0"],
    ["targetTicketsPerEpoch", "8192"],
    ["minAvailabilitySamples", "0"],
  ]) {
    assert.throws(
      () => buildUpdateConfigArgs({ [field]: value }, {}, config()),
      new RegExp(field),
    );
  }
  for (const redundancyBuffer of [0, 9, 1.5]) {
    assert.throws(
      () => validateProtocolConfig({ ...config(), redundancyBuffer }),
      /redundancyBuffer/,
    );
  }
});

test("epoch length and grace bounds", () => {
  for (const [options, pattern] of [
    [{ epochLenSeconds: "3599" }, /epochLenSeconds/],
    [{ epochLenSeconds: "2592001" }, /epochLenSeconds/],
    [{ ticketGraceSeconds: "86400" }, /ticketGraceSeconds/],
  ]) {
    assert.throws(() => buildUpdateConfigArgs(options, {}, config()), pattern);
  }
});

test("u64 and price overflow checks, and positive required fields", () => {
  for (const field of [
    "withdrawalCooldownSlots",
    "minimumNodeDeposit",
    "maxSettlementDelaySeconds",
  ]) {
    for (const value of ["0", "-1", "18446744073709551616"]) {
      assert.throws(
        () => buildUpdateConfigArgs({ [field]: value }, {}, config()),
        new RegExp(field),
      );
    }
  }
  assert.throws(
    () =>
      buildUpdateConfigArgs(
        { maxSettlementDelaySeconds: "604801" },
        {},
        config(),
      ),
    /604800/,
  );
  assert.throws(
    () =>
      buildUpdateConfigArgs(
        { rewardPerSignature: "18446744073709551615" },
        {},
        config(),
      ),
    /256 signers/,
  );
  assert.throws(
    () =>
      buildUpdateConfigArgs(
        { x402RoundBase: "18446744073709551615" },
        {},
        config(),
      ),
    /256 signers/,
  );
});

test("inclusive bounds and optional zero economics are accepted", () => {
  for (const low of [true, false]) {
    validateProtocolConfig({
      ...config(),
      epochLenSeconds: new BN(low ? 3600 : 2592000),
      ticketGraceSeconds: new BN(low ? 1 : 2591999),
      targetTicketsPerEpoch: low ? 1 : 8191,
      minAvailabilityBps: low ? 0 : 10000,
      minAvailabilitySamples: low ? 1 : 65535,
      protocolFeeBps: low ? 0 : 10000,
      challengerBountyBps: 0,
      rewardPerSignature: new BN(0),
      x402RoundBase: new BN(0),
    });
  }
});
