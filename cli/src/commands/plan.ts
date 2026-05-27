import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import { findPlanPda, findProtocolConfigPda } from "../lib/pdas";
import { bn, parseInteger, requireValue, writeJson } from "../lib/utils";

export function registerPlanCommands(program: Command): void {
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const args = buildPlanArgs(options);
      const planId = planTypeId(args.planType);
      const [planPda] = findPlanPda(ctx.programId, planId);

      const signature = await (ctx.program.methods as any)
        .addPlan(args)
        .accountsPartial({
          protocolConfig,
          plan: planPda,
          authority: ctx.wallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      writeJson({
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const args = buildPlanArgs(options);
      const planId = planTypeId(args.planType);
      const [planPda] = findPlanPda(ctx.programId, planId);

      const signature = await (ctx.program.methods as any)
        .updatePlan(args)
        .accountsPartial({
          protocolConfig,
          plan: planPda,
          authority: ctx.wallet.publicKey,
        })
        .rpc();

      writeJson({
        signature,
        planId,
        plan: planPda.toBase58(),
      });
    });
}

function buildPlanArgs(options: Record<string, string | boolean>) {
  return {
    planType: planTypeToAnchor(requireValue(options.planType as string, "Missing --plan-type")),
    subscriptionPrice: bn(
      requireValue(options.subscriptionPrice as string, "Missing --subscription-price"),
      "subscription price"
    ),
    maxJobs: parseInteger(requireValue(options.maxJobs as string, "Missing --max-jobs"), "max jobs"),
    signaturesRequired: 0,
    maxSigners: parseInteger(
      requireValue(options.maxSigners as string, "Missing --max-signers"),
      "max signers"
    ),
    privateApiEnabled: Boolean(options.privateApiEnabled),
    isActive: !options.inactive,
  };
}

function planTypeId(planType: Record<string, object>) {
  if ("basic" in planType) return 0;
  if ("standard" in planType) return 1;
  if ("professional" in planType) return 2;
  if ("enterprise" in planType) return 3;
  throw new Error("Unknown plan type");
}

function planTypeToAnchor(input: string) {
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
