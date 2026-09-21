import { validateProtocolConfig } from "../lib/protocol-config";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import {
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
  disputeWindowSlots?: string;
  gatewayBondMin?: string;
  challengerBountyBps?: number;
  livelinessFreezeThreshold?: number;
  livelinessPenaltyPerMiss?: number;
  livelinessRecoveryPerSign?: number;
  usdcMint?: string;
  x402RoundBase?: string;
  minSigners?: number;
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
      "max age of canonical_timestamp at settlement ingress",
    )
    .option("--dispute-window-slots <slots>", "settled round dispute window")
    .option("--gateway-bond-min <amount>", "minimum gateway bond")
    .option("--challenger-bounty-bps <bps>", "challenger bounty basis points")
    .option(
      "--liveliness-freeze-threshold <score>",
      "liveliness score below which a node may be frozen",
    )
    .option("--usdc-mint <pubkey>", "USDC mint")
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
        disputeWindowSlots: bn(
          requireValue(
            optionOrConfig(
              options.disputeWindowSlots,
              config.disputeWindowSlots,
            ),
            "Missing --dispute-window-slots",
          ),
          "dispute window",
        ),
        gatewayBondMin: bn(
          requireValue(
            optionOrConfig(options.gatewayBondMin, config.gatewayBondMin),
            "Missing --gateway-bond-min",
          ),
          "gateway bond minimum",
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
        livelinessFreezeThreshold: Number(
          requireValue(
            optionOrConfig(
              options.livelinessFreezeThreshold,
              config.livelinessFreezeThreshold,
            ),
            "Missing --liveliness-freeze-threshold",
          ),
        ),
        livelinessPenaltyPerMiss: Number(
          optionOrConfig(
            options.livelinessPenaltyPerMiss,
            config.livelinessPenaltyPerMiss,
          ) ?? 500,
        ),
        livelinessRecoveryPerSign: Number(
          optionOrConfig(
            options.livelinessRecoveryPerSign,
            config.livelinessRecoveryPerSign,
          ) ?? 100,
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

      const signature = await (ctx.program.methods as any)
        .initialize(args)
        .accountsPartial({
          protocolConfig,
          registryState,
          registry,
          usdcMint: args.usdcMint,
          treasury,
          nodeDepositVault,
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
