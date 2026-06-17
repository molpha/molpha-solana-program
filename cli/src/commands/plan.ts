import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { accountJson, fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findPlanPda, findProtocolConfigPda } from "../lib/pdas";
import {
  bn,
  parseInteger,
  parsePublicKey,
  requireValue,
  writeJson,
} from "../lib/utils";

export function registerPlanCommands(program: Command): void {
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const planId = planTypeId(
        planTypeToAnchor(
          requireValue(options.planType as string, "Missing --plan-type")
        )
      );
      const [planPda] = findPlanPda(ctx.programId, planId);

      const current = await fetchAccountOrNull(ctx.program, "plan", planPda);
      if (!current) {
        throw new Error(`Plan not found at ${planPda.toBase58()}`);
      }

      const args = buildUpdatePlanArgs(options, current);

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

  plan
    .command("show")
    .description("Fetch current on-chain plan data")
    .option("--plan-type <type>", "basic|standard|professional|enterprise")
    .option(
      "--plan-id <number>",
      "plan id (0=basic, 1=standard, 2=professional, 3=enterprise)"
    )
    .option("--address <pubkey>", "plan account address")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const { planPda, planId } = resolvePlanTarget(ctx.programId, options);

      const account = await fetchAccountOrNull(ctx.program, "plan", planPda);
      if (!account) {
        throw new Error(`Plan not found at ${planPda.toBase58()}`);
      }

      const resolvedPlanId = planId ?? planTypeId(account.planType);

      writeJson({
        ...(accountJson(planPda, account) as Record<string, unknown>),
        planId: resolvedPlanId,
        planType: planTypeName(account.planType),
      });
    });
}

function buildPlanArgs(options: Record<string, string | boolean>) {
  return {
    planType: planTypeToAnchor(
      requireValue(options.planType as string, "Missing --plan-type")
    ),
    subscriptionPrice: bn(
      requireValue(
        options.subscriptionPrice as string,
        "Missing --subscription-price"
      ),
      "subscription price"
    ),
    maxJobs: parseInteger(
      requireValue(options.maxJobs as string, "Missing --max-jobs"),
      "max jobs"
    ),
    signaturesRequired: 0,
    maxSigners: parseInteger(
      requireValue(options.maxSigners as string, "Missing --max-signers"),
      "max signers"
    ),
    privateApiEnabled: Boolean(options.privateApiEnabled),
    isActive: !options.inactive,
  };
}

function buildUpdatePlanArgs(
  options: Record<string, string | boolean | undefined>,
  current: {
    planType: Record<string, object>;
    subscriptionPrice: { toString(): string };
    maxJobs: number;
    maxSigners: number;
    privateApiEnabled: boolean;
    isActive: boolean;
  }
) {
  return {
    planType: planTypeToAnchor(
      requireValue(options.planType as string, "Missing --plan-type")
    ),
    subscriptionPrice:
      options.subscriptionPrice !== undefined
        ? bn(options.subscriptionPrice as string, "subscription price")
        : bn(current.subscriptionPrice.toString(), "subscription price"),
    maxJobs:
      options.maxJobs !== undefined
        ? parseInteger(options.maxJobs as string, "max jobs")
        : current.maxJobs,
    signaturesRequired: 0,
    maxSigners:
      options.maxSigners !== undefined
        ? parseInteger(options.maxSigners as string, "max signers")
        : current.maxSigners,
    privateApiEnabled:
      options.privateApiEnabled === true ? true : current.privateApiEnabled,
    isActive: options.inactive === true ? false : current.isActive,
  };
}

function planTypeId(planType: Record<string, object>) {
  if ("basic" in planType) return 0;
  if ("standard" in planType) return 1;
  if ("professional" in planType) return 2;
  if ("enterprise" in planType) return 3;
  throw new Error("Unknown plan type");
}

function resolvePlanTarget(
  programId: PublicKey,
  options: { planType?: string; planId?: string; address?: string }
): { planPda: PublicKey; planId?: number } {
  if (options.address) {
    return { planPda: parsePublicKey(options.address, "plan address") };
  }
  if (options.planType) {
    const planId = planTypeId(planTypeToAnchor(options.planType));
    const [planPda] = findPlanPda(programId, planId);
    return { planPda, planId };
  }
  if (options.planId !== undefined) {
    const planId = parseInteger(options.planId, "plan id");
    const [planPda] = findPlanPda(programId, planId);
    return { planPda, planId };
  }
  throw new Error("Provide --plan-type, --plan-id, or --address");
}

function planTypeName(planType: Record<string, object> | number): string {
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
