import { Command } from "commander";
import { getCliContext } from "../lib/client";
import { findJobPda } from "../lib/pdas";
import { hexToBytes, parsePublicKey, writeJson } from "../lib/utils";

export function registerDelegateCommands(program: Command): void {
  const delegate = program
    .command("delegate")
    .description("Manage job delegates");

  delegate
    .command("add")
    .description("Add a delegate to a job")
    .requiredOption("--job-id <hex>", "job id")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command: Command) => {
      await mutateDelegate("addDelegate", options, command);
    });

  delegate
    .command("remove")
    .description("Remove a delegate from a job")
    .requiredOption("--job-id <hex>", "job id")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command: Command) => {
      await mutateDelegate("removeDelegate", options, command);
    });
}

async function mutateDelegate(
  methodName: "addDelegate" | "removeDelegate",
  options: { jobId: string; delegate: string },
  command: Command
) {
  const ctx = await getCliContext(command);
  const jobId = hexToBytes(options.jobId, 32, "job id");
  const delegate = parsePublicKey(options.delegate, "delegate");
  const [jobPda] = findJobPda(ctx.programId, jobId);

  const signature = await (ctx.program.methods as any)
    [methodName]({
      jobId: Array.from(jobId),
      delegate,
    })
    .accountsPartial({
      owner: ctx.wallet.publicKey,
      job: jobPda,
    })
    .rpc();

  writeJson({
    signature,
    job: jobPda.toBase58(),
    delegate: delegate.toBase58(),
  });
}
