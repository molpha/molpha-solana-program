#!/usr/bin/env node

import { Command } from "commander";
import { registerAccountCommands } from "./commands/account";
import { registerAdminCommands } from "./commands/admin";
import { registerAttestationCommands } from "./commands/attestation";
import { registerDelegateCommands } from "./commands/delegate";
import { registerEpochCommands } from "./commands/epoch";
import { registerGatewayCommands } from "./commands/gateway";
import { registerInitCommand } from "./commands/init";
import { registerPlanCommands } from "./commands/plan";
import { registerRegistryCommands } from "./commands/registry";
import { registerRewardsCommands } from "./commands/rewards";
import { registerSlashCommands } from "./commands/slash";
import { registerSubscriptionCommands } from "./commands/subscription";

async function main() {
  const program = new Command();

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
      "confirmed",
    )
    .option("--idl <path>", "IDL file path override");

  registerAdminCommands(program);
  registerInitCommand(program);
  registerRegistryCommands(program);
  registerPlanCommands(program);
  registerSubscriptionCommands(program);
  registerDelegateCommands(program);
  registerGatewayCommands(program);
  registerEpochCommands(program);
  registerRewardsCommands(program);
  registerAttestationCommands(program);
  registerSlashCommands(program);
  registerAccountCommands(program);

  await program.parseAsync(process.argv);
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`${message}\n`);
  process.exit(1);
});
