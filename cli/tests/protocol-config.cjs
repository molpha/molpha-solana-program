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
    disputeWindowSlots: new BN(1024),
    gatewayBondMin: new BN(500),
    challengerBountyBps: 500,
    livelinessFreezeThreshold: 5000,
    livelinessPenaltyPerMiss: 750,
    livelinessRecoveryPerSign: 225,
    x402RoundBase: new BN(1000),
    minSigners: 3,
  };
}

test("partial updates preserve current rates and flags override JSON as numbers", () => {
  const current = config();
  assert.deepEqual(buildUpdateConfigArgs({}, {}, current), current);
  const args = buildUpdateConfigArgs(
    { livelinessPenaltyPerMiss: "800", minSigners: "4" },
    { livelinessPenaltyPerMiss: 900, livelinessRecoveryPerSign: 300 },
    current,
  );
  assert.equal(args.livelinessPenaltyPerMiss, 800);
  assert.equal(args.livelinessRecoveryPerSign, 300);
  assert.equal(args.minSigners, 4);
});

test("init and update commands expose both rate flags", () => {
  for (const register of [registerInitCommand, registerUpdateConfigCommand]) {
    const program = new Command();
    register(program);
    const flags = program.commands[0].options.map((option) => option.long);
    assert.ok(flags.includes("--liveliness-penalty-per-miss"));
    assert.ok(flags.includes("--liveliness-recovery-per-sign"));
  }
});

test("invalid integer knobs are rejected before encoding", () => {
  for (const field of [
    "livelinessPenaltyPerMiss",
    "livelinessRecoveryPerSign",
    "livelinessFreezeThreshold",
  ]) {
    for (const value of ["0", "-1", "10001", "1.5", "NaN", "Infinity"]) {
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

test("u64 and price overflow checks, and positive required fields", () => {
  for (const field of [
    "withdrawalCooldownSlots",
    "minimumNodeDeposit",
    "maxSettlementDelaySeconds",
    "disputeWindowSlots",
    "gatewayBondMin",
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
        { maxSettlementDelaySeconds: "9223372036854775808" },
        {},
        config(),
      ),
    /timestamp/,
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
  for (const score of [1, 10000]) {
    validateProtocolConfig({
      ...config(),
      livelinessFreezeThreshold: score,
      livelinessPenaltyPerMiss: score,
      livelinessRecoveryPerSign: score,
      challengerBountyBps: 0,
      rewardPerSignature: new BN(0),
      x402RoundBase: new BN(0),
    });
  }
});
