const assert = require("node:assert/strict");
const { test } = require("node:test");
const { BN } = require("@coral-xyz/anchor");
const { buildUpdateConfigArgs } = require("../dist/commands/update-config");
const {
  deriveInitialTau,
  validateInitialTau,
  validateProtocolConfig,
  TAU_MIN,
  U64_MAX,
} = require("../dist/lib/protocol-config");
const { Command } = require("commander");
const { Keypair, PublicKey } = require("@solana/web3.js");
const { registerInitCommand } = require("../dist/commands/init");
const {
  BPF_LOADER_UPGRADEABLE_PROGRAM_ID,
  decodeUpgradeAuthority,
  findProgramDataAddress,
  requireUpgradeAuthority,
} = require("../dist/lib/upgrade-authority");
const {
  registerUpdateConfigCommand,
} = require("../dist/commands/update-config");

const PROGRAM_ID = new PublicKey("chivcFQgxzwkpLvW41PV431HQ4dYpaW3povQH3AdpQt");

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

test("the genesis threshold is sized to the ticket target", () => {
  // 2,000 tickets out of 1M rounds: 2^64 * 2000 / 1e6.
  assert.equal(
    deriveInitialTau(2000, new BN(1_000_000)).toString(),
    new BN(1).shln(64).muln(2000).divn(1_000_000).toString(),
  );
  // No more rounds than the target: sample every round.
  assert.ok(deriveInitialTau(2000, new BN(2000)).eq(U64_MAX));
  assert.ok(deriveInitialTau(2000, new BN(5)).eq(U64_MAX));
  // A vanishing load clamps to the floor instead of underflowing the program's bound.
  assert.ok(deriveInitialTau(1, new BN("1000000000000000")).eq(TAU_MIN));
  assert.throws(() => deriveInitialTau(2000, new BN(0)), /expectedRounds/);
  validateInitialTau(TAU_MIN);
  validateInitialTau(U64_MAX);
  assert.throws(() => validateInitialTau(TAU_MIN.subn(1)), /initialTau/);
  assert.throws(() => validateInitialTau(U64_MAX.addn(1)), /initialTau/);
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
    if (register === registerInitCommand) {
      assert.ok(flags.includes("--initial-tau"));
      assert.ok(flags.includes("--expected-rounds-per-epoch"));
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

function programDataAccount(upgradeAuthority) {
  const data = Buffer.alloc(45 + 8);
  data.writeUInt32LE(3, 0);
  data.writeBigUInt64LE(42n, 4);
  if (upgradeAuthority) {
    data[12] = 1;
    upgradeAuthority.toBuffer().copy(data, 13);
  }
  return { owner: BPF_LOADER_UPGRADEABLE_PROGRAM_ID, data };
}

test("the ProgramData address and its upgrade authority are decoded as the loader lays them out", () => {
  // `solana find-program-derived-address BPFLoaderUpgradeab1e... pubkey:<program id>`
  assert.equal(
    findProgramDataAddress(PROGRAM_ID).toBase58(),
    "5dHqdjkaZGTKLtq7gkm52k5FhkR4dc7dpeAjKEGYcLAG",
  );
  const authority = Keypair.generate().publicKey;
  assert.ok(
    decodeUpgradeAuthority(programDataAccount(authority).data).equals(
      authority,
    ),
  );
  assert.equal(decodeUpgradeAuthority(programDataAccount(null).data), null);
  // A Program account (tag 2) or a truncated buffer is not ProgramData.
  const program = Buffer.alloc(45);
  program.writeUInt32LE(2, 0);
  assert.throws(() => decodeUpgradeAuthority(program), /ProgramData/);
  assert.throws(() => decodeUpgradeAuthority(Buffer.alloc(12)), /ProgramData/);
});

test("init refuses a wallet that is not the program's upgrade authority", async () => {
  const authority = Keypair.generate().publicKey;
  const other = Keypair.generate().publicKey;
  const cluster = (account) => ({
    getAccountInfo: async (address) => {
      assert.ok(address.equals(findProgramDataAddress(PROGRAM_ID)));
      return account;
    },
  });

  assert.ok(
    (
      await requireUpgradeAuthority(
        cluster(programDataAccount(authority)),
        PROGRAM_ID,
        authority,
      )
    ).equals(findProgramDataAddress(PROGRAM_ID)),
  );
  await assert.rejects(
    requireUpgradeAuthority(
      cluster(programDataAccount(authority)),
      PROGRAM_ID,
      other,
    ),
    new RegExp(`upgrade authority ${authority.toBase58()}`),
  );
  await assert.rejects(
    requireUpgradeAuthority(
      cluster(programDataAccount(null)),
      PROGRAM_ID,
      authority,
    ),
    /immutable/,
  );
  await assert.rejects(
    requireUpgradeAuthority(cluster(null), PROGRAM_ID, authority),
    /no ProgramData account/,
  );
  await assert.rejects(
    requireUpgradeAuthority(
      cluster({ ...programDataAccount(authority), owner: other }),
      PROGRAM_ID,
      authority,
    ),
    /no ProgramData account/,
  );
});
