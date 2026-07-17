import { BN } from "@coral-xyz/anchor";
import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findDelegatePda, findSubscriptionPda } from "../lib/pdas";
import { parseInteger, parsePublicKey, writeJson } from "../lib/utils";

export function registerDelegateCommands(program: Command): void {
  const delegate = program
    .command("delegate")
    .description("Manage owner-level delegates");

  delegate
    .command("add")
    .description(
      "Add a delegate with custom job/request limits (requires an active subscription)"
    )
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .requiredOption("--max-jobs <number>", "max jobs the delegate may create")
    .requiredOption(
      "--max-data-requests <number>",
      "max data requests the delegate may authorize"
    )
    .requiredOption(
      "--max-signers <number>",
      "per-delegate quorum ceiling (<= subscription.max_signers)"
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const delegateKey = parsePublicKey(options.delegate, "delegate");
      const [delegatePda] = findDelegatePda(
        ctx.programId,
        ctx.wallet.publicKey,
        delegateKey
      );
      const [subscriptionPda] = findSubscriptionPda(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const subscription = await fetchAccountOrNull(
        ctx.program,
        "subscription",
        subscriptionPda
      );
      if (!subscription) {
        throw new Error(
          `No subscription found at ${subscriptionPda.toBase58()} — subscribe first`
        );
      }

      const maxJobsCreated = parseInteger(options.maxJobs, "max jobs");
      const maxDataRequests = parseInteger(
        options.maxDataRequests,
        "max data requests"
      );
      const maxSigners = parseInteger(options.maxSigners, "max signers");

      const signature = await (ctx.program.methods as any)
        .addDelegate({
          delegate: delegateKey,
          maxJobsCreated,
          maxDataRequests: new BN(maxDataRequests),
          maxSigners,
        })
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          subscription: subscriptionPda,
          delegateAccount: delegatePda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      writeJson({
        signature,
        owner: ctx.wallet.publicKey.toBase58(),
        delegate: delegateKey.toBase58(),
        delegateAccount: delegatePda.toBase58(),
        subscription: subscriptionPda.toBase58(),
        maxJobsCreated,
        maxDataRequests,
        maxSigners,
      });
    });

  delegate
    .command("remove")
    .description("Remove a delegate and close its quota account")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const delegateKey = parsePublicKey(options.delegate, "delegate");
      const [delegatePda] = findDelegatePda(
        ctx.programId,
        ctx.wallet.publicKey,
        delegateKey
      );
      const [subscriptionPda] = findSubscriptionPda(
        ctx.programId,
        ctx.wallet.publicKey
      );

      const signature = await (ctx.program.methods as any)
        .removeDelegate({ delegate: delegateKey })
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          subscription: subscriptionPda,
          delegateAccount: delegatePda,
        })
        .rpc();

      writeJson({
        signature,
        owner: ctx.wallet.publicKey.toBase58(),
        delegate: delegateKey.toBase58(),
        delegateAccount: delegatePda.toBase58(),
        subscription: subscriptionPda.toBase58(),
      });
    });

  delegate
    .command("pause")
    .description("Pause or unpause a delegate")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .option("--paused", "pause the delegate (default)", true)
    .option("--unpaused", "unpause the delegate")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const delegateKey = parsePublicKey(options.delegate, "delegate");
      const [delegatePda] = findDelegatePda(
        ctx.programId,
        ctx.wallet.publicKey,
        delegateKey
      );
      const paused = options.unpaused ? false : true;

      const signature = await (ctx.program.methods as any)
        .setDelegatePaused(paused)
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          delegateAccount: delegatePda,
        })
        .rpc();

      writeJson({
        signature,
        owner: ctx.wallet.publicKey.toBase58(),
        delegate: delegateKey.toBase58(),
        delegateAccount: delegatePda.toBase58(),
        paused,
      });
    });
}
