"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerSubscriptionCommands = registerSubscriptionCommands;
const spl_token_1 = require("@solana/spl-token");
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("../lib/client");
const accounts_1 = require("../lib/accounts");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function planTypeToAnchor(planId) {
  const types = ["basic", "standard", "professional", "enterprise"];
  const name = types[planId];
  if (!name) {
    throw new Error(`Invalid plan id ${planId}; expected 0-3`);
  }
  return { [name]: {} };
}
function registerSubscriptionCommands(program) {
  const subscription = program
    .command("subscription")
    .description("Manage subscriptions");
  subscription
    .command("subscribe")
    .description("Create a monthly subscription")
    .requiredOption(
      "--plan-id <number>",
      "plan type id (0=basic, 1=standard, 2=professional, 3=enterprise)"
    )
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [protocolConfigPda] = (0, pdas_1.findProtocolConfigPda)(
        ctx.programId
      );
      const protocolConfig = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }
      const planId = (0, utils_1.parseInteger)(options.planId, "plan id");
      const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, planId);
      const [subscriptionPda] = (0, pdas_1.findSubscriptionPda)(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const ownerUsdc = options.ownerUsdc
        ? (0, utils_1.parsePublicKey)(options.ownerUsdc, "owner USDC account")
        : (0, spl_token_1.getAssociatedTokenAddressSync)(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );
      const signature = await ctx.program.methods
        .subscribe(planTypeToAnchor(planId))
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          plan: planPda,
          subscription: subscriptionPda,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          systemProgram: web3_js_1.SystemProgram.programId,
          tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
        })
        .rpc();
      (0, utils_1.writeJson)({
        signature,
        subscription: subscriptionPda.toBase58(),
      });
    });
  subscription
    .command("extend")
    .description("Extend an existing subscription by one month")
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [protocolConfigPda] = (0, pdas_1.findProtocolConfigPda)(
        ctx.programId
      );
      const protocolConfig = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }
      const [subscriptionPda] = (0, pdas_1.findSubscriptionPda)(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const subscriptionAccount = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "subscription",
        subscriptionPda
      );
      if (!subscriptionAccount) {
        throw new Error(
          `Subscription not found at ${subscriptionPda.toBase58()}`
        );
      }
      const planType = subscriptionAccount.planType;
      const planId =
        "basic" in planType
          ? 0
          : "standard" in planType
          ? 1
          : "professional" in planType
          ? 2
          : "enterprise" in planType
          ? 3
          : Number(subscriptionAccount.planType);
      const [planPda] = (0, pdas_1.findPlanPda)(ctx.programId, planId);
      const ownerUsdc = options.ownerUsdc
        ? (0, utils_1.parsePublicKey)(options.ownerUsdc, "owner USDC account")
        : (0, spl_token_1.getAssociatedTokenAddressSync)(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );
      const signature = await ctx.program.methods
        .extendSubscription()
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          subscription: subscriptionPda,
          plan: planPda,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
        })
        .rpc();
      (0, utils_1.writeJson)({
        signature,
        subscription: subscriptionPda.toBase58(),
      });
    });
}
