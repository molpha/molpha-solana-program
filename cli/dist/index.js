#!/usr/bin/env node
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const commander_1 = require("commander");
const account_1 = require("./commands/account");
const admin_1 = require("./commands/admin");
const data_1 = require("./commands/data");
const delegate_1 = require("./commands/delegate");
const init_1 = require("./commands/init");
const job_1 = require("./commands/job");
const plan_1 = require("./commands/plan");
const registry_1 = require("./commands/registry");
const subscription_1 = require("./commands/subscription");
async function main() {
  const program = new commander_1.Command();
  program
    .name("molpha")
    .description("TypeScript CLI for the Molpha Solana program")
    .version("0.1.0")
    .option("--cluster <name>", "localnet|devnet|mainnet", "localnet")
    .option("--rpc <url>", "RPC URL override")
    .option("--wallet <path>", "wallet keypair path")
    .option("--program-id <pubkey>", "Molpha program id override")
    .option(
      "--commitment <level>",
      "processed|confirmed|finalized",
      "confirmed"
    )
    .option("--idl <path>", "IDL file path override");
  (0, admin_1.registerAdminCommands)(program);
  (0, init_1.registerInitCommand)(program);
  (0, registry_1.registerRegistryCommands)(program);
  (0, plan_1.registerPlanCommands)(program);
  (0, subscription_1.registerSubscriptionCommands)(program);
  (0, job_1.registerJobCommands)(program);
  (0, delegate_1.registerDelegateCommands)(program);
  (0, data_1.registerDataCommands)(program);
  (0, account_1.registerAccountCommands)(program);
  await program.parseAsync(process.argv);
}
main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`${message}\n`);
  process.exit(1);
});
