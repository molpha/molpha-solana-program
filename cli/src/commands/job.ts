import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import { deriveJobId } from "../lib/ids";
import { fetchAccountOrNull } from "../lib/accounts";
import {
  findDelegatePda,
  findFeedPda,
  findJobPda,
  findPlanPda,
  findProtocolConfigPda,
  findSubscriptionPda,
} from "../lib/pdas";
import {
  hexToBytes,
  parseInteger,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

export function registerJobCommands(program: Command): void {
  const job = program.command("job").description("Manage jobs");

  job
    .command("create")
    .description("Create a new job and feed")
    .option("--owner <pubkey>", "job owner; defaults to wallet pubkey")
    .requiredOption("--api-config-hash <hex>", "32-byte api config hash")
    .requiredOption("--signatures-required <number>", "signatures required")
    .requiredOption("--decimals <number>", "decimals")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const jobOwner = options.owner
        ? parsePublicKey(options.owner, "owner")
        : ctx.wallet.publicKey;
      const creator = ctx.wallet.publicKey;
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [subscriptionPda] = findSubscriptionPda(ctx.programId, jobOwner);
      const subscription = await fetchAccountOrNull<any>(
        ctx.program,
        "subscription",
        subscriptionPda
      );
      if (!subscription) {
        throw new Error(
          `Subscription not found at ${subscriptionPda.toBase58()}`
        );
      }

      const [planPda] = findPlanPda(ctx.programId, Number(subscription.planId));
      const apiConfigHash = hexToBytes(
        options.apiConfigHash,
        32,
        "api config hash"
      );
      const jobId = deriveJobId(jobOwner, apiConfigHash);
      const [jobPda] = findJobPda(ctx.programId, jobId);
      const [feedPda] = findFeedPda(ctx.programId, jobId);

      const accounts: Record<string, unknown> = {
        creator,
        jobOwner,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        job: jobPda,
        subscription: subscriptionPda,
        feed: feedPda,
        delegate: null,
        systemProgram: SystemProgram.programId,
      };

      if (!creator.equals(jobOwner)) {
        const [delegatePda] = findDelegatePda(ctx.programId, jobOwner, creator);
        accounts.delegate = delegatePda;
      }

      const signature = await (ctx.program.methods as any)
        .createJob(
          {
            apiConfigHash: Array.from(apiConfigHash),
            signaturesRequired: parseInteger(
              options.signaturesRequired,
              "signatures required"
            ),
            decimals: parseInteger(options.decimals, "decimals"),
          },
          Array.from(jobId)
        )
        .accountsPartial(accounts)
        .rpc();

      writeJson({
        signature,
        owner: jobOwner.toBase58(),
        creator: creator.toBase58(),
        jobId: `0x${Buffer.from(jobId).toString("hex")}`,
        job: jobPda.toBase58(),
        feed: feedPda.toBase58(),
      });
    });
}
