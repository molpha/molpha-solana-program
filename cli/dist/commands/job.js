"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerJobCommands = registerJobCommands;
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("../lib/client");
const ids_1 = require("../lib/ids");
const accounts_1 = require("../lib/accounts");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerJobCommands(program) {
    const job = program.command("job").description("Manage jobs");
    job
        .command("create")
        .description("Create a new job and feed")
        .option("--owner <pubkey>", "job owner; defaults to wallet pubkey")
        .requiredOption("--api-config-hash <hex>", "32-byte api config hash")
        .requiredOption("--decimals <number>", "decimals")
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const owner = options.owner ? (0, utils_1.parsePublicKey)(options.owner, "owner") : ctx.wallet.publicKey;
        const [protocolConfigPda] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
        const [subscriptionPda] = (0, pdas_1.findSubscriptionPda)(ctx.programId, owner);
        const subscription = await (0, accounts_1.fetchAccountOrNull)(ctx.program, "subscription", subscriptionPda);
        if (!subscription) {
            throw new Error(`Subscription not found at ${subscriptionPda.toBase58()}`);
        }
        const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, Number(subscription.planId));
        const apiConfigHash = (0, utils_1.hexToBytes)(options.apiConfigHash, 32, "api config hash");
        const jobId = (0, ids_1.deriveJobId)(owner, apiConfigHash);
        const [jobPda] = (0, pdas_1.findJobPda)(ctx.programId, jobId);
        const [feedPda] = (0, pdas_1.findFeedPda)(ctx.programId, jobId);
        const signature = await ctx.program.methods
            .createJob({
            apiConfigHash: Array.from(apiConfigHash),
            decimals: (0, utils_1.parseInteger)(options.decimals, "decimals"),
        }, Array.from(jobId))
            .accountsPartial({
            owner,
            protocolConfig: protocolConfigPda,
            plan: planPda,
            job: jobPda,
            subscription: subscriptionPda,
            feed: feedPda,
            systemProgram: web3_js_1.SystemProgram.programId,
        })
            .rpc();
        (0, utils_1.writeJson)({
            signature,
            owner: owner.toBase58(),
            jobId: `0x${Buffer.from(jobId).toString("hex")}`,
            job: jobPda.toBase58(),
            feed: feedPda.toBase58(),
        });
    });
}
