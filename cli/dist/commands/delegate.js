"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerDelegateCommands = registerDelegateCommands;
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerDelegateCommands(program) {
  const delegate = program
    .command("delegate")
    .description("Manage job delegates");
  delegate
    .command("add")
    .description("Add a delegate to a job")
    .requiredOption("--job-id <hex>", "job id")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command) => {
      await mutateDelegate("addDelegate", options, command);
    });
  delegate
    .command("remove")
    .description("Remove a delegate from a job")
    .requiredOption("--job-id <hex>", "job id")
    .requiredOption("--delegate <pubkey>", "delegate pubkey")
    .action(async (options, command) => {
      await mutateDelegate("removeDelegate", options, command);
    });
}
async function mutateDelegate(methodName, options, command) {
  const ctx = await (0, client_1.getCliContext)(command);
  const jobId = (0, utils_1.hexToBytes)(options.jobId, 32, "job id");
  const delegate = (0, utils_1.parsePublicKey)(options.delegate, "delegate");
  const [jobPda] = (0, pdas_1.findJobPda)(ctx.programId, jobId);
  const signature = await ctx.program.methods[methodName]({
    jobId: Array.from(jobId),
    delegate,
  })
    .accountsPartial({
      owner: ctx.wallet.publicKey,
      job: jobPda,
    })
    .rpc();
  (0, utils_1.writeJson)({
    signature,
    job: jobPda.toBase58(),
    delegate: delegate.toBase58(),
  });
}
