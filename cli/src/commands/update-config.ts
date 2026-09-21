import { validateProtocolConfig } from "../lib/protocol-config";
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
  withdrawalCooldownSlots?: string;
  minimumNodeDeposit?: string;
  rewardPerSignature?: string;
  maxSettlementDelaySeconds?: string;
  disputeWindowSlots?: string;
  gatewayBondMin?: string;
  challengerBountyBps?: number;
  livelinessFreezeThreshold?: number;
  livelinessPenaltyPerMiss?: number;
  livelinessRecoveryPerSign?: number;
  x402RoundBase?: string;
  minSigners?: number;
};

/// Mirrors `ProtocolConfig`'s tunables. `reward_liability` is protocol-maintained accounting,
/// not a knob, so `update_config` neither reads nor writes it.
type ProtocolConfigAccount = {
  withdrawalCooldownSlots: { toString(): string };
  minimumNodeDeposit: { toString(): string };
  rewardPerSignature: { toString(): string };
  maxSettlementDelaySeconds: { toString(): string };
  disputeWindowSlots: { toString(): string };
  gatewayBondMin: { toString(): string };
  challengerBountyBps: number;
  livelinessFreezeThreshold: number;
  livelinessPenaltyPerMiss: number;
  livelinessRecoveryPerSign: number;
  x402RoundBase: { toString(): string };
  minSigners: number;
};

export function registerUpdateConfigCommand(admin: Command): void {
  admin
    .command("update-config")
    .description("Update protocol config (authority only)")
    .option("--config <path>", "json config file with fields to override")
    .option(
      "--withdrawal-cooldown-slots <slots>",
      "node withdrawal cooldown in slots",
    )
    .option("--minimum-node-deposit <amount>", "minimum USDC deposit")
    .option("--reward-per-signature <amount>", "reward per signature")
    .option(
      "--max-settlement-delay-seconds <seconds>",
      "max age of canonical_timestamp at settlement ingress",
    )
    .option("--dispute-window-slots <slots>", "settled round dispute window")
    .option("--gateway-bond-min <amount>", "minimum gateway bond")
    .option("--challenger-bounty-bps <bps>", "challenger bounty basis points")
    .option(
      "--liveliness-freeze-threshold <score>",
      "liveliness score below which a node may be frozen",
    )
    .option(
      "--liveliness-penalty-per-miss <score>",
      "score lost per missed round (1-10000)",
    )
    .option(
      "--liveliness-recovery-per-sign <score>",
      "score regained per signed round (1-10000)",
    )
    .option("--x402-round-base <amount>", "base x402 round settlement price")
    .option(
      "--min-signers <count>",
      "minimum signatures_required accepted by settlement",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const config = options.config
        ? readJsonFile<UpdateConfigFile>(options.config)
        : {};
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);

      const current = await fetchAccountOrNull<ProtocolConfigAccount>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda,
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

export function buildUpdateConfigArgs(
  options: Record<string, string | number | undefined>,
  config: UpdateConfigFile,
  current: ProtocolConfigAccount,
) {
  const args = {
    withdrawalCooldownSlots: bn(
      requireValue(
        optionOrConfig(
          options.withdrawalCooldownSlots as string | undefined,
          config.withdrawalCooldownSlots,
        ) ?? current.withdrawalCooldownSlots.toString(),
        "withdrawal cooldown",
      ),
      "withdrawal cooldown",
    ),
    minimumNodeDeposit: bn(
      requireValue(
        optionOrConfig(
          options.minimumNodeDeposit as string | undefined,
          config.minimumNodeDeposit,
        ) ?? current.minimumNodeDeposit.toString(),
        "minimum node deposit",
      ),
      "minimum node deposit",
    ),
    rewardPerSignature: bn(
      requireValue(
        optionOrConfig(
          options.rewardPerSignature as string | undefined,
          config.rewardPerSignature,
        ) ?? current.rewardPerSignature.toString(),
        "reward per signature",
      ),
      "reward per signature",
    ),
    maxSettlementDelaySeconds: bn(
      requireValue(
        optionOrConfig(
          options.maxSettlementDelaySeconds as string | undefined,
          config.maxSettlementDelaySeconds,
        ) ?? current.maxSettlementDelaySeconds.toString(),
        "max settlement delay",
      ),
      "max settlement delay",
    ),
    disputeWindowSlots: bn(
      requireValue(
        optionOrConfig(
          options.disputeWindowSlots as string | undefined,
          config.disputeWindowSlots,
        ) ?? current.disputeWindowSlots.toString(),
        "dispute window",
      ),
      "dispute window",
    ),
    gatewayBondMin: bn(
      requireValue(
        optionOrConfig(
          options.gatewayBondMin as string | undefined,
          config.gatewayBondMin,
        ) ?? current.gatewayBondMin.toString(),
        "gateway bond minimum",
      ),
      "gateway bond minimum",
    ),
    challengerBountyBps: Number(
      optionOrConfig(options.challengerBountyBps, config.challengerBountyBps) ??
        current.challengerBountyBps,
    ),
    livelinessFreezeThreshold: Number(
      optionOrConfig(
        options.livelinessFreezeThreshold,
        config.livelinessFreezeThreshold,
      ) ?? current.livelinessFreezeThreshold,
    ),
    livelinessPenaltyPerMiss: Number(
      optionOrConfig(
        options.livelinessPenaltyPerMiss,
        config.livelinessPenaltyPerMiss,
      ) ?? current.livelinessPenaltyPerMiss,
    ),
    livelinessRecoveryPerSign: Number(
      optionOrConfig(
        options.livelinessRecoveryPerSign,
        config.livelinessRecoveryPerSign,
      ) ?? current.livelinessRecoveryPerSign,
    ),
    x402RoundBase: bn(
      requireValue(
        optionOrConfig(
          options.x402RoundBase as string | undefined,
          config.x402RoundBase,
        ) ?? current.x402RoundBase.toString(),
        "x402 round base",
      ),
      "x402 round base",
    ),
    minSigners: Number(
      optionOrConfig(options.minSigners, config.minSigners) ??
        current.minSigners,
    ),
  };
  validateProtocolConfig(args);
  return args;
}
