import { Command } from "commander";
import { accountJson, fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findDelegatePda,
  findFeedPda,
  findGatewayPda,
  findNodePda,
  findPlanPda,
  findProtocolConfigPda,
  findRegistryPda,
  findRegistryStatePda,
  findSecp256k1KeyClaimPda,
  findSubscriptionPda,
} from "../lib/pdas";
import {
  hexToBytes,
  parseInteger,
  parsePublicKey,
  requireValue,
  writeJson,
} from "../lib/utils";

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
  const account = program
    .command("account")
    .description("Read Molpha accounts");

  account
    .command("protocol-config")
    .description("Fetch protocol config")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ?? findProtocolConfigPda(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "protocolConfig", address);
    });

  account
    .command("registry")
    .description("Fetch the current-version registry pointer")
    .option("--address <pubkey>", "override PDA")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ?? findRegistryStatePda(ctx.programId)[0].toBase58();
      await printNamedAccount(command, "registryState", address);
    });

  account
    .command("registry-version")
    .description("Fetch an immutable registry snapshot")
    .option("--address <pubkey>", "registry account address")
    .option("--version <number>", "registry version")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findRegistryPda(
          ctx.programId,
          parseInteger(
            requireValue(options.version, "Missing --version"),
            "version"
          )
        )[0].toBase58();
      await printNamedAccount(command, "registry", address);
    });

  account
    .command("node")
    .description("Fetch a permanent owner-keyed node account")
    .option("--address <pubkey>", "node account address")
    .option("--owner <pubkey>", "node owner")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findNodePda(
          ctx.programId,
          parsePublicKey(requireValue(options.owner, "Missing --owner"))
        )[0].toBase58();
      await printNamedAccount(command, "node", address);
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
        findPlanPda(
          ctx.programId,
          parseInteger(
            requireValue(options.planId, "Missing --plan-id"),
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.owner
        ? parsePublicKey(options.owner, "owner")
        : ctx.wallet.publicKey;
      const address =
        options.address ??
        findSubscriptionPda(ctx.programId, owner)[0].toBase58();
      await printNamedAccount(command, "subscription", address);
    });

  account
    .command("feed")
    .description("Fetch feed account")
    .option("--address <pubkey>", "feed address")
    .option("--source-id <hex>", "32-byte source identifier")
    .option(
      "--signatures-required <number>",
      "quorum used when the feed was created"
    )
    .option("--submitter <pubkey>", "feed submitter (defaults to wallet)")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address = options.address
        ? options.address
        : findFeedPda(
            ctx.programId,
            hexToBytes(
              requireValue(options.sourceId, "Missing --source-id"),
              32,
              "source id"
            ),
            parseInteger(
              requireValue(
                options.signaturesRequired,
                "Missing --signatures-required"
              ),
              "signatures required"
            ),
            options.submitter
              ? parsePublicKey(options.submitter, "submitter")
              : ctx.wallet.publicKey
          )[0].toBase58();
      await printNamedAccount(command, "feed", address);
    });

  account
    .command("delegate")
    .description("Fetch delegate account")
    .option("--address <pubkey>", "delegate account address")
    .option("--owner <pubkey>", "subscription owner")
    .option("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findDelegatePda(
          ctx.programId,
          parsePublicKey(
            requireValue(options.owner, "Missing --owner"),
            "owner"
          ),
          parsePublicKey(
            requireValue(options.delegate, "Missing --delegate"),
            "delegate"
          )
        )[0].toBase58();
      await printNamedAccount(command, "delegate", address);
    });

  account
    .command("gateway")
    .description("Fetch gateway account")
    .option("--address <pubkey>", "gateway address")
    .option("--authority <pubkey>", "gateway authority")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findGatewayPda(
          ctx.programId,
          parsePublicKey(
            requireValue(options.authority, "Missing --authority"),
            "authority"
          )
        )[0].toBase58();
      await printNamedAccount(command, "gateway", address);
    });


  account
    .command("round")
    .description("Fetch round receipt record by address")
    .requiredOption("--address <pubkey>", "round receipt record address")
    .action(async (options, command: Command) => {
      await printNamedAccount(command, "roundReceiptRecord", options.address);
    });

  account
    .command("secp-key")
    .description("Fetch secp256k1 key claim")
    .option("--address <pubkey>", "claim address")
    .option("--secp-pubkey <hex>", "33-byte compressed secp256k1 pubkey")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address =
        options.address ??
        findSecp256k1KeyClaimPda(
          ctx.programId,
          hexToBytes(
            requireValue(options.secpPubkey, "Missing --secp-pubkey"),
            33,
            "secp pubkey"
          )
        )[0].toBase58();
      await printNamedAccount(command, "secp256k1KeyClaim", address);
    });
}
