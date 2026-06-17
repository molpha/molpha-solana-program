"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerAccountCommands = registerAccountCommands;
const accounts_1 = require("../lib/accounts");
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
async function printNamedAccount(command, accountName, address) {
  const ctx = await (0, client_1.getCliContext)(command);
  const pubkey = (0, utils_1.parsePublicKey)(address);
  const account = await (0, accounts_1.fetchAccountOrNull)(
    ctx.program,
    accountName,
    pubkey
  );
  if (!account) {
    throw new Error(`${accountName} not found at ${pubkey.toBase58()}`);
  }
  (0, utils_1.writeJson)((0, accounts_1.accountJson)(pubkey, account));
}
function registerAccountCommands(program) {
  const account = program
    .command("account")
    .description("Read Molpha accounts");
  account
    .command("protocol-config")
    .description("Fetch protocol config")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        (0, pdas_1.findProtocolConfigPda)(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "protocolConfig", address);
    });
  account
    .command("registry")
    .description("Fetch registry state")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        (0, pdas_1.findRegistryStatePda)(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "registryState", address);
    });
  account
    .command("node")
    .description("Fetch node account")
    .option("--address <pubkey>", "node account address")
    .option("--node-id <hex>", "32-byte node id")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        findJobOrNodeAddress("node", ctx.programId, options.nodeId).toBase58();
      await printNamedAccount(command, "nodeAccount", address);
    });
  account
    .command("plan")
    .description("Fetch plan account")
    .option("--address <pubkey>", "plan address")
    .option("--plan-id <number>", "plan id")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        (0, pdas_1.findPlanPda)(
          ctx.programId,
          (0, utils_1.parseInteger)(
            (0, utils_1.requireValue)(options.planId, "Missing --plan-id"),
            "plan id"
          )
        )[0].toBase58();
      await printNamedAccount(command, "plan", address);
    });
  account
    .command("subscription")
    .description("Fetch subscription account")
    .option("--address <pubkey>", "subscription address")
    .option("--owner <pubkey>", "subscription owner")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const owner = options.owner
        ? (0, utils_1.parsePublicKey)(options.owner, "owner")
        : ctx.wallet.publicKey;
      const address =
        options.address ??
        (0, pdas_1.findSubscriptionPda)(ctx.programId, owner)[0].toBase58();
      await printNamedAccount(command, "subscription", address);
    });
  account
    .command("job")
    .description("Fetch job account")
    .option("--address <pubkey>", "job address")
    .option("--job-id <hex>", "32-byte job id")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        findJobOrNodeAddress("job", ctx.programId, options.jobId).toBase58();
      await printNamedAccount(command, "job", address);
    });
  account
    .command("feed")
    .description("Fetch feed account")
    .option("--address <pubkey>", "feed address")
    .option("--job-id <hex>", "32-byte job id")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const jobId = (0, utils_1.hexToBytes)(
        (0, utils_1.requireValue)(options.jobId, "Missing --job-id"),
        32,
        "job id"
      );
      const address =
        options.address ??
        (0, pdas_1.findFeedPda)(ctx.programId, jobId)[0].toBase58();
      await printNamedAccount(command, "feed", address);
    });
  account
    .command("registry-index")
    .description("Fetch registry index account")
    .option("--address <pubkey>", "registry index address")
    .option("--index <number>", "registry index")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const address =
        options.address ??
        (0, pdas_1.findRegistryIndexPda)(
          ctx.programId,
          (0, utils_1.parseInteger)(
            (0, utils_1.requireValue)(options.index, "Missing --index"),
            "index"
          )
        )[0].toBase58();
      await printNamedAccount(command, "registryIndexAccount", address);
    });
}
function findJobOrNodeAddress(kind, programId, idHex) {
  const label = kind === "job" ? "job" : "node";
  const bytes = (0, utils_1.hexToBytes)(
    (0, utils_1.requireValue)(idHex, `Missing --${label}-id`),
    32,
    `${label} id`
  );
  return kind === "job"
    ? (0, pdas_1.findJobPda)(programId, bytes)[0]
    : (0, pdas_1.findNodePda)(programId, bytes)[0];
}
