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
  challengerBountyBps?: number;
  x402RoundBase?: string;
  minSigners?: number;
  epochLenSeconds?: string;
  ticketGraceSeconds?: string;
  targetTicketsPerEpoch?: number;
  minAvailabilityBps?: number;
  minAvailabilitySamples?: number;
  protocolFeeBps?: number;
};

/// Mirrors `ProtocolConfig`'s tunables. `reward_liability`, `pool_liability` and
/// `protocol_reserved` are protocol-maintained accounting, not knobs, so `update_config`
/// neither reads nor writes them. Epoch parameters reach only epochs created afterwards.
type ProtocolConfigAccount = {
  withdrawalCooldownSlots: { toString(): string };
  minimumNodeDeposit: { toString(): string };
  rewardPerSignature: { toString(): string };
  maxSettlementDelaySeconds: { toString(): string };
  challengerBountyBps: number;
  x402RoundBase: { toString(): string };
  minSigners: number;
  epochLenSeconds: { toString(): string };
  ticketGraceSeconds: { toString(): string };
  targetTicketsPerEpoch: number;
  minAvailabilityBps: number;
  minAvailabilitySamples: number;
  protocolFeeBps: number;
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
      "max age of canonical_timestamp at attestation ingress; registry version grace",
    )
    .option("--challenger-bounty-bps <bps>", "challenger bounty basis points")
    .option(
      "--x402-round-base <amount>",
      "published x402 base price per round (pricing input only)",
    )
    .option(
      "--min-signers <count>",
      "minimum signatures_required accepted for attestations and tickets",
    )
    .option("--epoch-len-seconds <seconds>", "length of epochs created later")
    .option(
      "--ticket-grace-seconds <seconds>",
      "late-ticket window of epochs created later",
    )
    .option(
      "--target-tickets-per-epoch <count>",
      "ticket count the sampling threshold aims for",
    )
    .option(
      "--min-availability-bps <bps>",
      "payout floor relative to the network inclusion rate",
    )
    .option(
      "--min-availability-samples <count>",
      "selections below which the floor does not apply",
    )
    .option(
      "--protocol-fee-bps <bps>",
      "share of swept income kept by the protocol; the rest funds epoch pools",
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
    challengerBountyBps: Number(
      optionOrConfig(options.challengerBountyBps, config.challengerBountyBps) ??
        current.challengerBountyBps,
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
    epochLenSeconds: bn(
      optionOrConfig(
        options.epochLenSeconds as string | undefined,
        config.epochLenSeconds,
      ) ?? current.epochLenSeconds.toString(),
      "epoch length",
    ),
    ticketGraceSeconds: bn(
      optionOrConfig(
        options.ticketGraceSeconds as string | undefined,
        config.ticketGraceSeconds,
      ) ?? current.ticketGraceSeconds.toString(),
      "ticket grace",
    ),
    targetTicketsPerEpoch: Number(
      optionOrConfig(
        options.targetTicketsPerEpoch,
        config.targetTicketsPerEpoch,
      ) ?? current.targetTicketsPerEpoch,
    ),
    minAvailabilityBps: Number(
      optionOrConfig(options.minAvailabilityBps, config.minAvailabilityBps) ??
        current.minAvailabilityBps,
    ),
    minAvailabilitySamples: Number(
      optionOrConfig(
        options.minAvailabilitySamples,
        config.minAvailabilitySamples,
      ) ?? current.minAvailabilitySamples,
    ),
    protocolFeeBps: Number(
      optionOrConfig(options.protocolFeeBps, config.protocolFeeBps) ??
        current.protocolFeeBps,
    ),
  };
  validateProtocolConfig(args);
  return args;
}
