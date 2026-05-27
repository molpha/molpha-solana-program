import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import { findProtocolConfigPda, findRegistryStatePda } from "../lib/pdas";
import { bn, optionOrConfig, parsePublicKey, readJsonFile, requireValue, writeJson } from "../lib/utils";

type InitConfig = {
  registryCooldownSecs?: string;
  previousGraceSecs?: string;
  withdrawalCooldownSecs?: string;
  maxNodeCount?: number;
  redundancyBuffer?: number;
  minimumNodeDeposit?: string;
  rewardPerSignature?: string;
  gatewayFee?: string;
  receiptSubmitterBonus?: string;
  protocolFeeBps?: number;
  maxReceiptDelaySlots?: string;
  usdcMint?: string;
};

export function registerInitCommand(program: Command): void {
  program
    .command("init")
    .description("Initialize the Molpha protocol")
    .option("--config <path>", "json config file")
    .option("--registry-cooldown-secs <seconds>", "registry cooldown")
    .option("--previous-grace-secs <seconds>", "previous registry grace period")
    .option("--withdrawal-cooldown-secs <seconds>", "node withdrawal cooldown")
    .option("--max-node-count <count>", "max registry size")
    .option("--redundancy-buffer <count>", "extra selection slots beyond signatures_required")
    .option("--minimum-node-deposit <amount>", "minimum USDC deposit")
    .option("--reward-per-signature <amount>", "reward per signature")
    .option("--gateway-fee <amount>", "gateway fee")
    .option("--receipt-submitter-bonus <amount>", "receipt submitter bonus")
    .option("--protocol-fee-bps <bps>", "protocol fee in basis points")
    .option("--max-receipt-delay-slots <slots>", "max receipt delay")
    .option("--usdc-mint <pubkey>", "USDC mint")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const config = options.config ? readJsonFile<InitConfig>(options.config) : {};
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [registryState] = findRegistryStatePda(ctx.programId);

      const args = {
        registryCooldownSecs: bn(
          requireValue(
            optionOrConfig(options.registryCooldownSecs, config.registryCooldownSecs),
            "Missing --registry-cooldown-secs"
          ),
          "registry cooldown"
        ),
        previousGraceSecs: bn(
          requireValue(
            optionOrConfig(options.previousGraceSecs, config.previousGraceSecs),
            "Missing --previous-grace-secs"
          ),
          "previous grace"
        ),
        withdrawalCooldownSecs: bn(
          requireValue(
            optionOrConfig(options.withdrawalCooldownSecs, config.withdrawalCooldownSecs),
            "Missing --withdrawal-cooldown-secs"
          ),
          "withdrawal cooldown"
        ),
        maxNodeCount: Number(
          requireValue(optionOrConfig(options.maxNodeCount, config.maxNodeCount), "Missing --max-node-count")
        ),
        redundancyBuffer: Number(
          optionOrConfig(options.redundancyBuffer, config.redundancyBuffer) ?? 2
        ),
        minimumNodeDeposit: bn(
          requireValue(
            optionOrConfig(options.minimumNodeDeposit, config.minimumNodeDeposit),
            "Missing --minimum-node-deposit"
          ),
          "minimum node deposit"
        ),
        rewardPerSignature: bn(
          requireValue(
            optionOrConfig(options.rewardPerSignature, config.rewardPerSignature),
            "Missing --reward-per-signature"
          ),
          "reward per signature"
        ),
        gatewayFee: bn(
          requireValue(optionOrConfig(options.gatewayFee, config.gatewayFee), "Missing --gateway-fee"),
          "gateway fee"
        ),
        receiptSubmitterBonus: bn(
          requireValue(
            optionOrConfig(options.receiptSubmitterBonus, config.receiptSubmitterBonus),
            "Missing --receipt-submitter-bonus"
          ),
          "receipt submitter bonus"
        ),
        protocolFeeBps: Number(
          requireValue(optionOrConfig(options.protocolFeeBps, config.protocolFeeBps), "Missing --protocol-fee-bps")
        ),
        maxReceiptDelaySlots: bn(
          requireValue(
            optionOrConfig(options.maxReceiptDelaySlots, config.maxReceiptDelaySlots),
            "Missing --max-receipt-delay-slots"
          ),
          "max receipt delay"
        ),
        usdcMint: parsePubkeyOption(optionOrConfig(options.usdcMint, config.usdcMint), "usdc mint"),
      };
      const treasury = getAssociatedTokenAddressSync(args.usdcMint, protocolConfig, true);

      const signature = await (ctx.program.methods as any)
        .initialize(args)
        .accountsPartial({
          protocolConfig,
          registryState,
          usdcMint: args.usdcMint,
          treasury,
          authority: ctx.wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      writeJson({
        signature,
        protocolConfig: protocolConfig.toBase58(),
        registryState: registryState.toBase58(),
        treasury: treasury.toBase58(),
      });
    });
}

function parsePubkeyOption(value: string | undefined, label: string): PublicKey {
  return parsePublicKey(requireValue(value, `Missing --${label.replace(/\s+/g, "-")}`), label);
}
