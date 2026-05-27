import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import { fetchAccountOrNull } from "../lib/accounts";
import { findPlanPda, findProtocolConfigPda, findSubscriptionPda } from "../lib/pdas";
import { parseInteger, parsePublicKey, requireValue, writeJson } from "../lib/utils";

function planTypeToAnchor(planId: number) {
  const types = ["basic", "standard", "professional", "enterprise"] as const;
  const name = types[planId];
  if (!name) {
    throw new Error(`Invalid plan id ${planId}; expected 0-3`);
  }
  return { [name]: {} };
}

export function registerSubscriptionCommands(program: Command): void {
  const subscription = program
    .command("subscription")
    .description("Manage subscriptions");

  subscription
    .command("subscribe")
    .description("Create a monthly subscription")
    .requiredOption("--plan-id <number>", "plan type id (0=basic, 1=standard, 2=professional, 3=enterprise)")
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(ctx.program, "protocolConfig", protocolConfigPda);
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const planId = parseInteger(options.planId, "plan id");
      const [planPda] = findPlanPda(ctx.programId, planId);
      const [subscriptionPda] = findSubscriptionPda(ctx.programId, ctx.wallet.publicKey);
      const ownerUsdc =
        options.ownerUsdc
          ? parsePublicKey(options.ownerUsdc, "owner USDC account")
          : getAssociatedTokenAddressSync(protocolConfig.usdcMint, ctx.wallet.publicKey);

      const signature = await (ctx.program.methods as any)
        .subscribe(planTypeToAnchor(planId))
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          plan: planPda,
          subscription: subscriptionPda,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      writeJson({
        signature,
        subscription: subscriptionPda.toBase58(),
      });
    });

  subscription
    .command("extend")
    .description("Extend an existing subscription by one month")
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(ctx.program, "protocolConfig", protocolConfigPda);
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const [subscriptionPda] = findSubscriptionPda(ctx.programId, ctx.wallet.publicKey);
      const subscriptionAccount = await fetchAccountOrNull<any>(
        ctx.program,
        "subscription",
        subscriptionPda
      );
      if (!subscriptionAccount) {
        throw new Error(`Subscription not found at ${subscriptionPda.toBase58()}`);
      }

      const planType = subscriptionAccount.planType;
      const planId =
        "basic" in planType ? 0 :
        "standard" in planType ? 1 :
        "professional" in planType ? 2 :
        "enterprise" in planType ? 3 :
        Number(subscriptionAccount.planType);
      const [planPda] = findPlanPda(ctx.programId, planId);
      const ownerUsdc =
        options.ownerUsdc
          ? parsePublicKey(options.ownerUsdc, "owner USDC account")
          : getAssociatedTokenAddressSync(protocolConfig.usdcMint, ctx.wallet.publicKey);

      const signature = await (ctx.program.methods as any)
        .extendSubscription()
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          subscription: subscriptionPda,
          plan: planPda,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      writeJson({
        signature,
        subscription: subscriptionPda.toBase58(),
      });
    });
}
