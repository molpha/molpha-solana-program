import { BN } from "@coral-xyz/anchor";
import { Command } from "commander";
import { SystemProgram } from "@solana/web3.js";
import { getCliContext } from "../lib/client";
import { findDelegatePda } from "../lib/pdas";
import { parseInteger, parsePublicKey, writeJson } from "../lib/utils";

export function registerDelegateCommands(program: Command): void {
  const delegate = program
    .command("delegate")
    .description("Manage owner-level delegates");

  delegate
    .command("add")
    .description("Add a delegate with custom job/request limits")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .requiredOption("--max-jobs <number>", "max jobs the delegate may create")
    .requiredOption(
      "--max-data-requests <number>",
      "max data requests the delegate may authorize"
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

async function mutateDelegate(
  methodName: "addDelegate" | "removeDelegate",
  options: {
    delegate: string;
    maxJobs?: string;
    maxDataRequests?: string;
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

  if (methodName === "addDelegate") {
    const maxJobsCreated = parseInteger(options.maxJobs!, "max jobs");
    const maxDataRequests = parseInteger(
      options.maxDataRequests!,
      "max data requests"
    );

    const signature = await (ctx.program.methods as any)
      .addDelegate({
        delegate,
        maxJobsCreated,
        maxDataRequests: new BN(maxDataRequests),
      })
      .accountsPartial({
        owner: ctx.wallet.publicKey,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    writeJson({
      signature,
      owner: ctx.wallet.publicKey.toBase58(),
      delegate: delegate.toBase58(),
      delegateAccount: delegatePda.toBase58(),
      maxJobsCreated,
      maxDataRequests,
    });
    return;
  }

  const signature = await (ctx.program.methods as any)
    .removeDelegate({ delegate })
    .accountsPartial({
      owner: ctx.wallet.publicKey,
      delegateAccount: delegatePda,
    })
    .rpc();

  writeJson({
    signature,
    owner: ctx.wallet.publicKey.toBase58(),
    delegate: delegate.toBase58(),
    delegateAccount: delegatePda.toBase58(),
  });
}
