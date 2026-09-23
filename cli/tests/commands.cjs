const assert = require("node:assert/strict");
const { test } = require("node:test");
const { Command } = require("commander");
const { registerGatewayCommands } = require("../dist/commands/gateway");
const { registerRoundCommands } = require("../dist/commands/round");

test("gateway CLI exposes only registration and deactivation", () => {
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

test("round CLI exposes immediate settlement commands that require aggregate signatures", () => {
  const program = new Command();
  registerRoundCommands(program);
  const round = program.commands.find((command) => command.name() === "round");
  assert.deepEqual(
    round.commands.map((command) => command.name()),
    ["settle-subscription", "settle-x402", "close-replay"],
  );
  for (const command of round.commands.filter(
    (c) => c.name() !== "close-replay",
  )) {
    const aggregateSignature = command.options.find(
      (option) => option.long === "--agg-sig",
    );
    assert.equal(aggregateSignature.required, true);
  }
});
