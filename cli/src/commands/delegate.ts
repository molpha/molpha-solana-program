import { BN } from "@coral-xyz/anchor";
import { Command } from "commander";
import { SystemProgram } from "@solana/web3.js";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findDelegatePda, findPlanPda, findSubscriptionPda } from "../lib/pdas";
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
      "per-delegate quorum ceiling (<= plan.max_signers)"
    )
    .action(async (options, command: Command) => {
      await mutateDelegate("addDelegate", options, command);
    });

  delegate
    .command("remove")
    .description("Remove a delegate and close its quota account")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command: Command) => {
      await mutateDelegate("removeDelegate", options, command);
    });
}

function planTypeId(planType: Record<string, object>): number {
  if ("basic" in planType) return 0;
  if ("standard" in planType) return 1;
  if ("professional" in planType) return 2;
  if ("enterprise" in planType) return 3;
  throw new Error("Unknown plan type");
}

async function mutateDelegate(
  methodName: "addDelegate" | "removeDelegate",
  options: {
    delegate: string;
    maxJobs?: string;
    maxDataRequests?: string;
    maxSigners?: string;
  },
  command: Command
) {
  const ctx = await getCliContext(command);
  const delegate = parsePublicKey(options.delegate, "delegate");
  const [delegatePda] = findDelegatePda(
    ctx.programId,
    ctx.wallet.publicKey,
    delegate
  );
  const [subscriptionPda] = findSubscriptionPda(
    ctx.programId,
    ctx.wallet.publicKey
  );

  if (methodName === "addDelegate") {
    const maxJobsCreated = parseInteger(options.maxJobs!, "max jobs");
    const maxDataRequests = parseInteger(
      options.maxDataRequests!,
      "max data requests"
    );
    const maxSigners = parseInteger(options.maxSigners!, "max signers");

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
    const [planPda] = findPlanPda(
      ctx.programId,
      planTypeId(subscription.planType)
    );

    const signature = await (ctx.program.methods as any)
      .addDelegate({
        delegate,
        maxJobsCreated,
        maxDataRequests: new BN(maxDataRequests),
        maxSigners,
      })
      .accountsPartial({
        owner: ctx.wallet.publicKey,
        subscription: subscriptionPda,
        plan: planPda,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    writeJson({
      signature,
      owner: ctx.wallet.publicKey.toBase58(),
      delegate: delegate.toBase58(),
      delegateAccount: delegatePda.toBase58(),
      subscription: subscriptionPda.toBase58(),
      maxJobsCreated,
      maxDataRequests,
      maxSigners,
    });
    return;
  }

  const signature = await (ctx.program.methods as any)
    .removeDelegate({ delegate })
    .accountsPartial({
      owner: ctx.wallet.publicKey,
      subscription: subscriptionPda,
      delegateAccount: delegatePda,
    })
    .rpc();

  writeJson({
    signature,
    owner: ctx.wallet.publicKey.toBase58(),
    delegate: delegate.toBase58(),
    delegateAccount: delegatePda.toBase58(),
    subscription: subscriptionPda.toBase58(),
  });
}
