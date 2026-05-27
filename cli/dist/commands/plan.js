"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerPlanCommands = registerPlanCommands;
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerPlanCommands(program) {
    const plan = program.command("plan").description("Manage Molpha plans");
    plan
        .command("add")
        .description("Create a plan")
        .requiredOption("--plan-type <type>", "basic|standard|professional|enterprise")
        .requiredOption("--subscription-price <amount>", "monthly subscription price in USDC base units")
        .requiredOption("--max-jobs <count>", "max jobs")
        .requiredOption("--max-signers <count>", "max signers")
        .option("--private-api-enabled", "enable private api")
        .option("--inactive", "create the plan inactive")
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const [protocolConfig] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
        const args = buildPlanArgs(options);
        const planId = planTypeId(args.planType);
        const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, planId);
        const signature = await ctx.program.methods
            .addPlan(args)
            .accountsPartial({
            protocolConfig,
            plan: planPda,
            authority: ctx.wallet.publicKey,
            systemProgram: web3_js_1.SystemProgram.programId,
        })
            .rpc();
        (0, utils_1.writeJson)({
            signature,
            planId,
            plan: planPda.toBase58(),
        });
    });
    plan
        .command("update")
        .description("Update a plan")
        .requiredOption("--plan-type <type>", "basic|standard|professional|enterprise")
        .requiredOption("--subscription-price <amount>", "monthly subscription price in USDC base units")
        .requiredOption("--max-jobs <count>", "max jobs")
        .requiredOption("--max-signers <count>", "max signers")
        .option("--private-api-enabled", "enable private api")
        .option("--inactive", "set inactive")
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const [protocolConfig] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
        const args = buildPlanArgs(options);
        const planId = planTypeId(args.planType);
        const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, planId);
        const signature = await ctx.program.methods
            .updatePlan(args)
            .accountsPartial({
            protocolConfig,
            plan: planPda,
            authority: ctx.wallet.publicKey,
        })
            .rpc();
        (0, utils_1.writeJson)({
            signature,
            planId,
            plan: planPda.toBase58(),
        });
    });
}
function buildPlanArgs(options) {
    return {
        planType: planTypeToAnchor((0, utils_1.requireValue)(options.planType, "Missing --plan-type")),
        subscriptionPrice: (0, utils_1.bn)((0, utils_1.requireValue)(options.subscriptionPrice, "Missing --subscription-price"), "subscription price"),
        maxJobs: (0, utils_1.parseInteger)((0, utils_1.requireValue)(options.maxJobs, "Missing --max-jobs"), "max jobs"),
        signaturesRequired: 0,
        maxSigners: (0, utils_1.parseInteger)((0, utils_1.requireValue)(options.maxSigners, "Missing --max-signers"), "max signers"),
        privateApiEnabled: Boolean(options.privateApiEnabled),
        isActive: !options.inactive,
    };
}
function planTypeId(planType) {
    if ("basic" in planType)
        return 0;
    if ("standard" in planType)
        return 1;
    if ("professional" in planType)
        return 2;
    if ("enterprise" in planType)
        return 3;
    throw new Error("Unknown plan type");
}
function planTypeToAnchor(input) {
    const value = input.trim().toLowerCase();
    if (value === "basic") {
        return { basic: {} };
    }
    if (value === "standard") {
        return { standard: {} };
    }
    if (value === "professional") {
        return { professional: {} };
    }
    if (value === "enterprise") {
        return { enterprise: {} };
    }
    throw new Error(`Unsupported plan type: ${input}`);
}
