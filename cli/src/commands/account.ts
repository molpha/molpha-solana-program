import { Command } from "commander";
import {
  accountJson,
  fetchAccountOrNull,
} from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findFeedPda,
  findJobPda,
  findNodePda,
  findPlanPda,
  findProtocolConfigPda,
  findRegistryIndexPda,
  findRegistryStatePda,
  findRegistryTransitionPda,
  findSubscriptionPda,
} from "../lib/pdas";
import { hexToBytes, parseInteger, parsePublicKey, requireValue, writeJson } from "../lib/utils";

async function printNamedAccount(
  command: Command,
  accountName: string,
  address: Parameters<typeof parsePublicKey>[0]
) {
  const ctx = await getCliContext(command);
  const pubkey = parsePublicKey(address);
  const account = await fetchAccountOrNull(ctx.program, accountName, pubkey);
  if (!account) {
    throw new Error(`${accountName} not found at ${pubkey.toBase58()}`);
  }
  writeJson(accountJson(pubkey, account));
}

export function registerAccountCommands(program: Command): void {
  const account = program.command("account").description("Read Molpha accounts");

  account
    .command("protocol-config")
    .description("Fetch protocol config")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address = options.address ?? findProtocolConfigPda(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "protocolConfig", address);
    });

  account
    .command("registry")
    .description("Fetch registry state")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address = options.address ?? findRegistryStatePda(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "registryState", address);
    });

  account
    .command("node")
    .description("Fetch node account")
    .option("--address <pubkey>", "node account address")
    .option("--node-id <hex>", "32-byte node id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findPlanPda(ctx.programId, parseInteger(requireValue(options.planId, "Missing --plan-id"), "plan id"))[0].toBase58();
      await printNamedAccount(command, "plan", address);
    });

  account
    .command("subscription")
    .description("Fetch subscription account")
    .option("--address <pubkey>", "subscription address")
    .option("--owner <pubkey>", "subscription owner")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.owner ? parsePublicKey(options.owner, "owner") : ctx.wallet.publicKey;
      const address =
        options.address ?? findSubscriptionPda(ctx.programId, owner)[0].toBase58();
      await printNamedAccount(command, "subscription", address);
    });

  account
    .command("job")
    .description("Fetch job account")
    .option("--address <pubkey>", "job address")
    .option("--job-id <hex>", "32-byte job id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ?? findJobOrNodeAddress("job", ctx.programId, options.jobId).toBase58();
      await printNamedAccount(command, "job", address);
    });

  account
    .command("feed")
    .description("Fetch feed account")
    .option("--address <pubkey>", "feed address")
    .option("--job-id <hex>", "32-byte job id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const jobId = hexToBytes(requireValue(options.jobId, "Missing --job-id"), 32, "job id");
      const address = options.address ?? findFeedPda(ctx.programId, jobId)[0].toBase58();
      await printNamedAccount(command, "feed", address);
    });

  account
    .command("registry-index")
    .description("Fetch registry index account")
    .option("--address <pubkey>", "registry index address")
    .option("--index <number>", "registry index")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findRegistryIndexPda(
          ctx.programId,
          parseInteger(requireValue(options.index, "Missing --index"), "index")
        )[0].toBase58();
      await printNamedAccount(command, "registryIndexAccount", address);
    });

  account
    .command("registry-transition")
    .description("Fetch registry transition account")
    .option("--address <pubkey>", "transition address")
    .option("--previous-version <number>", "previous version used in PDA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findRegistryTransitionPda(
          ctx.programId,
          parseInteger(
            requireValue(options.previousVersion, "Missing --previous-version"),
            "previous version"
          )
        )[0].toBase58();
      await printNamedAccount(command, "registryTransitionAccount", address);
    });
}

function findJobOrNodeAddress(kind: "job" | "node", programId: ReturnType<typeof parsePublicKey>, idHex: string) {
  const label = kind === "job" ? "job" : "node";
  const bytes = hexToBytes(requireValue(idHex, `Missing --${label}-id`), 32, `${label} id`);
  return kind === "job" ? findJobPda(programId, bytes)[0] : findNodePda(programId, bytes)[0];
}
