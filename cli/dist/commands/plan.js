"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerPlanCommands = registerPlanCommands;
const web3_js_1 = require("@solana/web3.js");
const accounts_1 = require("../lib/accounts");
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerPlanCommands(program) {
  const plan = program.command("plan").description("Manage Molpha plans");
  plan
    .command("add")
    .description("Create a plan")
    .requiredOption(
      "--plan-type <type>",
      "basic|standard|professional|enterprise"
    )
    .requiredOption(
      "--subscription-price <amount>",
      "monthly subscription price in USDC base units"
    )
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
    .requiredOption(
      "--plan-type <type>",
      "basic|standard|professional|enterprise"
    )
    .option(
      "--subscription-price <amount>",
      "monthly subscription price in USDC base units"
    )
    .option("--max-jobs <count>", "max jobs")
    .option("--max-signers <count>", "max signers")
    .option("--private-api-enabled", "enable private api")
    .option("--inactive", "set inactive")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [protocolConfig] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
      const planId = planTypeId(
        planTypeToAnchor(
          (0, utils_1.requireValue)(options.planType, "Missing --plan-type")
        )
      );
      const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, planId);
      const current = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "plan",
        planPda
      );
      if (!current) {
        throw new Error(`Plan not found at ${planPda.toBase58()}`);
      }
      const args = buildUpdatePlanArgs(options, current);
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
  plan
    .command("show")
    .description("Fetch current on-chain plan data")
    .option("--plan-type <type>", "basic|standard|professional|enterprise")
    .option(
      "--plan-id <number>",
      "plan id (0=basic, 1=standard, 2=professional, 3=enterprise)"
    )
    .option("--address <pubkey>", "plan account address")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const { planPda, planId } = resolvePlanTarget(ctx.programId, options);
      const account = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "plan",
        planPda
      );
      if (!account) {
        throw new Error(`Plan not found at ${planPda.toBase58()}`);
      }
      const resolvedPlanId = planId ?? planTypeId(account.planType);
      (0, utils_1.writeJson)({
        ...(0, accounts_1.accountJson)(planPda, account),
        planId: resolvedPlanId,
        planType: planTypeName(account.planType),
      });
    });
}
function buildPlanArgs(options) {
  return {
    planType: planTypeToAnchor(
      (0, utils_1.requireValue)(options.planType, "Missing --plan-type")
    ),
    subscriptionPrice: (0, utils_1.bn)(
      (0, utils_1.requireValue)(
        options.subscriptionPrice,
        "Missing --subscription-price"
      ),
      "subscription price"
    ),
    maxJobs: (0, utils_1.parseInteger)(
      (0, utils_1.requireValue)(options.maxJobs, "Missing --max-jobs"),
      "max jobs"
    ),
    signaturesRequired: 0,
    maxSigners: (0, utils_1.parseInteger)(
      (0, utils_1.requireValue)(options.maxSigners, "Missing --max-signers"),
      "max signers"
    ),
    privateApiEnabled: Boolean(options.privateApiEnabled),
    isActive: !options.inactive,
  };
}
function buildUpdatePlanArgs(options, current) {
  return {
    planType: planTypeToAnchor(
      (0, utils_1.requireValue)(options.planType, "Missing --plan-type")
    ),
    subscriptionPrice:
      options.subscriptionPrice !== undefined
        ? (0, utils_1.bn)(options.subscriptionPrice, "subscription price")
        : (0, utils_1.bn)(
            current.subscriptionPrice.toString(),
            "subscription price"
          ),
    maxJobs:
      options.maxJobs !== undefined
        ? (0, utils_1.parseInteger)(options.maxJobs, "max jobs")
        : current.maxJobs,
    signaturesRequired: 0,
    maxSigners:
      options.maxSigners !== undefined
        ? (0, utils_1.parseInteger)(options.maxSigners, "max signers")
        : current.maxSigners,
    privateApiEnabled:
      options.privateApiEnabled === true ? true : current.privateApiEnabled,
    isActive: options.inactive === true ? false : current.isActive,
  };
}
function planTypeId(planType) {
  if ("basic" in planType) return 0;
  if ("standard" in planType) return 1;
  if ("professional" in planType) return 2;
  if ("enterprise" in planType) return 3;
  throw new Error("Unknown plan type");
}
function resolvePlanTarget(programId, options) {
  if (options.address) {
    return {
      planPda: (0, utils_1.parsePublicKey)(options.address, "plan address"),
    };
  }
  if (options.planType) {
    const planId = planTypeId(planTypeToAnchor(options.planType));
    const [planPda] = (0, pdas_1.findPlanPda)(programId, planId);
    return { planPda, planId };
  }
  if (options.planId !== undefined) {
    const planId = (0, utils_1.parseInteger)(options.planId, "plan id");
    const [planPda] = (0, pdas_1.findPlanPda)(programId, planId);
    return { planPda, planId };
  }
  throw new Error("Provide --plan-type, --plan-id, or --address");
}
function planTypeName(planType) {
  if (typeof planType === "number") {
    const names = ["basic", "standard", "professional", "enterprise"];
    return names[planType] ?? String(planType);
  }
  if ("basic" in planType) return "basic";
  if ("standard" in planType) return "standard";
  if ("professional" in planType) return "professional";
  if ("enterprise" in planType) return "enterprise";
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
