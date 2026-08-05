import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findProtocolConfigPda } from "../lib/pdas";
import {
  bn,
  optionOrConfig,
  readJsonFile,
  requireValue,
  writeJson,
} from "../lib/utils";

type UpdateConfigFile = {
  withdrawalCooldownSecs?: string;
  maxNodeCount?: number;
  minimumNodeDeposit?: string;
  rewardPerSignature?: string;
  gatewayFee?: string;
  receiptSubmitterBonus?: string;
  protocolFeeBps?: number;
  maxReceiptDelaySlots?: string;
  protocolFee?: string;
  disputeWindowSlots?: string;
  gatewayBondMin?: string;
  challengerBountyBps?: number;
  agentRoundBase?: string;
  agentRoundPerSig?: string;
  agentEpochLenSecs?: string;
};

type ProtocolConfigAccount = {
  withdrawalCooldownSecs: { toString(): string };
  maxNodeCount: number;
  minimumNodeDeposit: { toString(): string };
  rewardPerSignature: { toString(): string };
  gatewayFee: { toString(): string };
  receiptSubmitterBonus: { toString(): string };
  protocolFeeBps: number;
  maxReceiptDelaySlots: { toString(): string };
  protocolFee: { toString(): string };
  disputeWindowSlots: { toString(): string };
  gatewayBondMin: { toString(): string };
  challengerBountyBps: number;
  agentRoundBase: { toString(): string };
  agentRoundPerSig: { toString(): string };
  agentEpochLenSecs: { toString(): string };
};

export function registerUpdateConfigCommand(admin: Command): void {
  admin
    .command("update-config")
    .description("Update protocol config (authority only)")
    .option("--config <path>", "json config file with fields to override")
    .option("--withdrawal-cooldown-secs <seconds>", "node withdrawal cooldown")
    .option("--max-node-count <count>", "max registry size")
    .option("--minimum-node-deposit <amount>", "minimum USDC deposit")
    .option("--reward-per-signature <amount>", "reward per signature")
    .option("--gateway-fee <amount>", "gateway fee")
    .option("--receipt-submitter-bonus <amount>", "receipt submitter bonus")
    .option("--protocol-fee-bps <bps>", "protocol fee in basis points")
    .option("--max-receipt-delay-slots <slots>", "max receipt delay")
    .option("--protocol-fee <amount>", "fixed protocol fee per round")
    .option("--dispute-window-slots <slots>", "settled round dispute window")
    .option("--gateway-bond-min <amount>", "minimum gateway bond")
    .option("--challenger-bounty-bps <bps>", "challenger bounty basis points")
    .option("--agent-round-base <amount>", "base agent fresh-round price")
    .option(
      "--agent-round-per-sig <amount>",
      "per-required-signature agent fresh-round price"
    )
    .option("--agent-epoch-len-secs <seconds>", "agent spend accounting epoch")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const config = options.config
        ? readJsonFile<UpdateConfigFile>(options.config)
        : {};
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);

      const current = await fetchAccountOrNull<ProtocolConfigAccount>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!current) {
        throw new Error("Protocol config is not initialized");
      }

      const args = buildUpdateConfigArgs(options, config, current);

      const signature = await (ctx.program.methods as any)
        .updateConfig(args)
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          authority: ctx.wallet.publicKey,
        })
        .rpc();

      writeJson({
        signature,
        protocolConfig: protocolConfigPda.toBase58(),
      });
    });
}

function buildUpdateConfigArgs(
  options: Record<string, string | number | undefined>,
  config: UpdateConfigFile,
  current: ProtocolConfigAccount
) {
  return {
    withdrawalCooldownSecs: bn(
      requireValue(
        optionOrConfig(
          options.withdrawalCooldownSecs as string | undefined,
          config.withdrawalCooldownSecs
        ) ?? current.withdrawalCooldownSecs.toString(),
        "withdrawal cooldown"
      ),
      "withdrawal cooldown"
    ),
    maxNodeCount:
      optionOrConfig(
        options.maxNodeCount as number | undefined,
        config.maxNodeCount
      ) ?? current.maxNodeCount,
    minimumNodeDeposit: bn(
      requireValue(
        optionOrConfig(
          options.minimumNodeDeposit as string | undefined,
          config.minimumNodeDeposit
        ) ?? current.minimumNodeDeposit.toString(),
        "minimum node deposit"
      ),
      "minimum node deposit"
    ),
    rewardPerSignature: bn(
      requireValue(
        optionOrConfig(
          options.rewardPerSignature as string | undefined,
          config.rewardPerSignature
        ) ?? current.rewardPerSignature.toString(),
        "reward per signature"
      ),
      "reward per signature"
    ),
    gatewayFee: bn(
      requireValue(
        optionOrConfig(
          options.gatewayFee as string | undefined,
          config.gatewayFee
        ) ?? current.gatewayFee.toString(),
        "gateway fee"
      ),
      "gateway fee"
    ),
    receiptSubmitterBonus: bn(
      requireValue(
        optionOrConfig(
          options.receiptSubmitterBonus as string | undefined,
          config.receiptSubmitterBonus
        ) ?? current.receiptSubmitterBonus.toString(),
        "receipt submitter bonus"
      ),
      "receipt submitter bonus"
    ),
    protocolFeeBps:
      optionOrConfig(
        options.protocolFeeBps as number | undefined,
        config.protocolFeeBps
      ) ?? current.protocolFeeBps,
    maxReceiptDelaySlots: bn(
      requireValue(
        optionOrConfig(
          options.maxReceiptDelaySlots as string | undefined,
          config.maxReceiptDelaySlots
        ) ?? current.maxReceiptDelaySlots.toString(),
        "max receipt delay"
      ),
      "max receipt delay"
    ),
    protocolFee: bn(
      requireValue(
        optionOrConfig(
          options.protocolFee as string | undefined,
          config.protocolFee
        ) ?? current.protocolFee.toString(),
        "protocol fee"
      ),
      "protocol fee"
    ),
    disputeWindowSlots: bn(
      requireValue(
        optionOrConfig(
          options.disputeWindowSlots as string | undefined,
          config.disputeWindowSlots
        ) ?? current.disputeWindowSlots.toString(),
        "dispute window"
      ),
      "dispute window"
    ),
    gatewayBondMin: bn(
      requireValue(
        optionOrConfig(
          options.gatewayBondMin as string | undefined,
          config.gatewayBondMin
        ) ?? current.gatewayBondMin.toString(),
        "gateway bond minimum"
      ),
      "gateway bond minimum"
    ),
    challengerBountyBps:
      optionOrConfig(
        options.challengerBountyBps as number | undefined,
        config.challengerBountyBps
      ) ?? current.challengerBountyBps,
    agentRoundBase: bn(
      requireValue(
        optionOrConfig(
          options.agentRoundBase as string | undefined,
          config.agentRoundBase
        ) ?? current.agentRoundBase.toString(),
        "agent round base"
      ),
      "agent round base"
    ),
    agentRoundPerSig: bn(
      requireValue(
        optionOrConfig(
          options.agentRoundPerSig as string | undefined,
          config.agentRoundPerSig
        ) ?? current.agentRoundPerSig.toString(),
        "agent round per signature"
      ),
      "agent round per signature"
    ),
    agentEpochLenSecs: bn(
      requireValue(
        optionOrConfig(
          options.agentEpochLenSecs as string | undefined,
          config.agentEpochLenSecs
        ) ?? current.agentEpochLenSecs.toString(),
        "agent epoch length"
      ),
      "agent epoch length"
    ),
  };
}
