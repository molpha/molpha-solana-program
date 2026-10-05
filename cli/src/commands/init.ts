import { EPOCH_DEFAULTS, validateProtocolConfig } from "../lib/protocol-config";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import {
  findEpochPda,
  findNodeDepositVault,
  findProtocolConfigPda,
  findRegistryPda,
  findRegistryStatePda,
} from "../lib/pdas";
import {
  bn,
  optionOrConfig,
  parsePublicKey,
  readJsonFile,
  requireValue,
  writeJson,
} from "../lib/utils";

type InitConfig = {
  withdrawalCooldownSlots?: string;
  redundancyBuffer?: number;
  minimumNodeDeposit?: string;
  rewardPerSignature?: string;
  maxSettlementDelaySeconds?: string;
  challengerBountyBps?: number;
  usdcMint?: string;
  x402RoundBase?: string;
  minSigners?: number;
  epochLenSeconds?: string;
  ticketGraceSeconds?: string;
  targetTicketsPerEpoch?: number;
  minAvailabilityBps?: number;
  minAvailabilitySamples?: number;
  protocolFeeBps?: number;
};

export function registerInitCommand(program: Command): void {
  program
    .command("init")
    .description("Initialize the Molpha protocol")
    .option("--config <path>", "json config file")
    .option(
      "--withdrawal-cooldown-slots <slots>",
      "node withdrawal cooldown in slots",
    )
    .option(
      "--redundancy-buffer <count>",
      "extra selection slots beyond signatures_required",
    )
    .option("--minimum-node-deposit <amount>", "minimum USDC deposit")
    .option("--reward-per-signature <amount>", "reward per signature")
    .option(
      "--max-settlement-delay-seconds <seconds>",
      "max age of timestamp at attestation ingress; registry version grace",
    )
    .option("--challenger-bounty-bps <bps>", "challenger bounty basis points")
    .option("--usdc-mint <pubkey>", "USDC mint")
    .option(
      "--x402-round-base <amount>",
      "published x402 base price per round (pricing input only)",
    )
    .option(
      "--min-signers <count>",
      "minimum signatures_required accepted for attestations and tickets",
    )
    .option("--epoch-len-seconds <seconds>", "epoch length (default 86400)")
    .option(
      "--ticket-grace-seconds <seconds>",
      "late-ticket window after an epoch ends (default 21600)",
    )
    .option(
      "--target-tickets-per-epoch <count>",
      "ticket count the sampling threshold aims for (default 2000)",
    )
    .option(
      "--min-availability-bps <bps>",
      "payout floor relative to the network inclusion rate (default 8000)",
    )
    .option(
      "--min-availability-samples <count>",
      "selections below which the floor does not apply (default 50)",
    )
    .option(
      "--protocol-fee-bps <bps>",
      "share of swept income kept by the protocol; the rest funds epoch pools",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const config = options.config
        ? readJsonFile<InitConfig>(options.config)
        : {};
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const [registry] = findRegistryPda(ctx.programId, 0);

      const args = {
        withdrawalCooldownSlots: bn(
          requireValue(
            optionOrConfig(
              options.withdrawalCooldownSlots,
              config.withdrawalCooldownSlots,
            ),
            "Missing --withdrawal-cooldown-slots",
          ),
          "withdrawal cooldown",
        ),
        redundancyBuffer: Number(
          optionOrConfig(options.redundancyBuffer, config.redundancyBuffer) ??
            2,
        ),
        minimumNodeDeposit: bn(
          requireValue(
            optionOrConfig(
              options.minimumNodeDeposit,
              config.minimumNodeDeposit,
            ),
            "Missing --minimum-node-deposit",
          ),
          "minimum node deposit",
        ),
        rewardPerSignature: bn(
          requireValue(
            optionOrConfig(
              options.rewardPerSignature,
              config.rewardPerSignature,
            ),
            "Missing --reward-per-signature",
          ),
          "reward per signature",
        ),
        maxSettlementDelaySeconds: bn(
          requireValue(
            optionOrConfig(
              options.maxSettlementDelaySeconds,
              config.maxSettlementDelaySeconds,
            ),
            "Missing --max-settlement-delay-seconds",
          ),
          "max settlement delay",
        ),
        challengerBountyBps: Number(
          requireValue(
            optionOrConfig(
              options.challengerBountyBps,
              config.challengerBountyBps,
            ),
            "Missing --challenger-bounty-bps",
          ),
        ),
        usdcMint: parsePubkeyOption(
          optionOrConfig(options.usdcMint, config.usdcMint),
          "usdc mint",
        ),
        x402RoundBase: bn(
          requireValue(
            optionOrConfig(options.x402RoundBase, config.x402RoundBase),
            "Missing --x402-round-base",
          ),
          "x402 round base",
        ),
        minSigners: Number(
          optionOrConfig(options.minSigners, config.minSigners) ?? 3,
        ),
        epochLenSeconds: bn(
          optionOrConfig(options.epochLenSeconds, config.epochLenSeconds) ??
            EPOCH_DEFAULTS.epochLenSeconds,
          "epoch length",
        ),
        ticketGraceSeconds: bn(
          optionOrConfig(
            options.ticketGraceSeconds,
            config.ticketGraceSeconds,
          ) ?? EPOCH_DEFAULTS.ticketGraceSeconds,
          "ticket grace",
        ),
        targetTicketsPerEpoch: Number(
          optionOrConfig(
            options.targetTicketsPerEpoch,
            config.targetTicketsPerEpoch,
          ) ?? EPOCH_DEFAULTS.targetTicketsPerEpoch,
        ),
        minAvailabilityBps: Number(
          optionOrConfig(
            options.minAvailabilityBps,
            config.minAvailabilityBps,
          ) ?? EPOCH_DEFAULTS.minAvailabilityBps,
        ),
        minAvailabilitySamples: Number(
          optionOrConfig(
            options.minAvailabilitySamples,
            config.minAvailabilitySamples,
          ) ?? EPOCH_DEFAULTS.minAvailabilitySamples,
        ),
        // The fee level is an economic decision: no default.
        protocolFeeBps: Number(
          requireValue(
            optionOrConfig(options.protocolFeeBps, config.protocolFeeBps),
            "Missing --protocol-fee-bps",
          ),
        ),
      };
      validateProtocolConfig(args);
      const treasury = getAssociatedTokenAddressSync(
        args.usdcMint,
        protocolConfig,
        true,
      );
      const nodeDepositVault = findNodeDepositVault(
        ctx.programId,
        args.usdcMint,
      );
      const [epoch0] = findEpochPda(ctx.programId, 0);
      const [epoch1] = findEpochPda(ctx.programId, 1);

      const signature = await (ctx.program.methods as any)
        .initialize(args)
        .accountsPartial({
          protocolConfig,
          registryState,
          registry,
          usdcMint: args.usdcMint,
          treasury,
          nodeDepositVault,
          epoch0,
          epoch1,
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
        registry: registry.toBase58(),
        treasury: treasury.toBase58(),
        nodeDepositVault: nodeDepositVault.toBase58(),
        epoch0: epoch0.toBase58(),
        epoch1: epoch1.toBase58(),
      });
    });
}

function parsePubkeyOption(
  value: string | undefined,
  label: string,
): PublicKey {
  return parsePublicKey(
    requireValue(value, `Missing --${label.replace(/\s+/g, "-")}`),
    label,
  );
}
