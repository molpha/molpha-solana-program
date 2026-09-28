const assert = require("node:assert/strict");
const { test } = require("node:test");
const { Command } = require("commander");
const { registerGatewayCommands } = require("../dist/commands/gateway");
const { registerEpochCommands } = require("../dist/commands/epoch");

test("gateway CLI exposes registration and deactivation only", () => {
  const program = new Command();
  registerGatewayCommands(program);
  const gateway = program.commands.find(
    (command) => command.name() === "gateway",
  );
  assert.deepEqual(
    gateway.commands.map((command) => command.name()),
    ["register", "deactivate"],
  );
  const register = gateway.commands.find(
    (command) => command.name() === "register",
  );
  const flags = register.options.map((option) => option.long);
  assert.ok(flags.includes("--gateway-authority"));
  assert.ok(!flags.includes("--bond-amount"));
});

test("epoch reward commands replace per-round settlement", () => {
  const program = new Command();
  registerEpochCommands(program);
  const groups = Object.fromEntries(
    program.commands.map((group) => [
      group.name(),
      group.commands.map((command) => command.name()),
    ]),
  );
  assert.deepEqual(groups, {
    epoch: ["show", "finalize", "credit", "close"],
    ticket: ["submit", "close"],
    revenue: ["sweep-treasury", "withdraw-reserve"],
  });
  const submit = program.commands
    .find((group) => group.name() === "ticket")
    .commands.find((command) => command.name() === "submit");
  for (const flag of ["--agg-sig", "--epoch", "--gateway-authority"]) {
    const option = submit.options.find((o) => o.long === flag);
    assert.equal(option.required, true, flag);
  }
});
