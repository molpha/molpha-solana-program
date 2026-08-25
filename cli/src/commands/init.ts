import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { getCliContext } from "../lib/client";
import {
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
  withdrawalCooldownSecs?: string;
  maxNodeCount?: number;
  redundancyBuffer?: number;
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
  usdcMint?: string;
  x402RoundBase?: string;
  x402RoundPerSig?: string;
};

export function registerInitCommand(program: Command): void {
  program
    .command("init")
    .description("Initialize the Molpha protocol")
    .option("--config <path>", "json config file")
    .option("--withdrawal-cooldown-secs <seconds>", "node withdrawal cooldown")
    .option("--max-node-count <count>", "max registry size")
    .option(
      "--redundancy-buffer <count>",
      "extra selection slots beyond signatures_required"
    )
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
    .option("--usdc-mint <pubkey>", "USDC mint")
    .option("--x402-round-base <amount>", "base x402 round settlement price")
    .option(
      "--x402-round-per-sig <amount>",
      "per-required-signature x402 round settlement price"
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
        withdrawalCooldownSecs: bn(
          requireValue(
            optionOrConfig(
              options.withdrawalCooldownSecs,
              config.withdrawalCooldownSecs
            ),
            "Missing --withdrawal-cooldown-secs"
          ),
          "withdrawal cooldown"
        ),
        maxNodeCount: Number(
          requireValue(
            optionOrConfig(options.maxNodeCount, config.maxNodeCount),
            "Missing --max-node-count"
          )
        ),
        redundancyBuffer: Number(
          optionOrConfig(options.redundancyBuffer, config.redundancyBuffer) ?? 2
        ),
        minimumNodeDeposit: bn(
          requireValue(
            optionOrConfig(
              options.minimumNodeDeposit,
              config.minimumNodeDeposit
            ),
            "Missing --minimum-node-deposit"
          ),
          "minimum node deposit"
        ),
        rewardPerSignature: bn(
          requireValue(
            optionOrConfig(
              options.rewardPerSignature,
              config.rewardPerSignature
            ),
            "Missing --reward-per-signature"
          ),
          "reward per signature"
        ),
        gatewayFee: bn(
          requireValue(
            optionOrConfig(options.gatewayFee, config.gatewayFee),
            "Missing --gateway-fee"
          ),
          "gateway fee"
        ),
        receiptSubmitterBonus: bn(
          requireValue(
            optionOrConfig(
              options.receiptSubmitterBonus,
              config.receiptSubmitterBonus
            ),
            "Missing --receipt-submitter-bonus"
          ),
          "receipt submitter bonus"
        ),
        protocolFeeBps: Number(
          requireValue(
            optionOrConfig(options.protocolFeeBps, config.protocolFeeBps),
            "Missing --protocol-fee-bps"
          )
        ),
        maxReceiptDelaySlots: bn(
          requireValue(
            optionOrConfig(
              options.maxReceiptDelaySlots,
              config.maxReceiptDelaySlots
            ),
            "Missing --max-receipt-delay-slots"
          ),
          "max receipt delay"
        ),
        protocolFee: bn(
          requireValue(
            optionOrConfig(options.protocolFee, config.protocolFee),
            "Missing --protocol-fee"
          ),
          "protocol fee"
        ),
        disputeWindowSlots: bn(
          requireValue(
            optionOrConfig(
              options.disputeWindowSlots,
              config.disputeWindowSlots
            ),
            "Missing --dispute-window-slots"
          ),
          "dispute window"
        ),
        gatewayBondMin: bn(
          requireValue(
            optionOrConfig(options.gatewayBondMin, config.gatewayBondMin),
            "Missing --gateway-bond-min"
          ),
          "gateway bond minimum"
        ),
        challengerBountyBps: Number(
          requireValue(
            optionOrConfig(
              options.challengerBountyBps,
              config.challengerBountyBps
            ),
            "Missing --challenger-bounty-bps"
          )
        ),
        usdcMint: parsePubkeyOption(
          optionOrConfig(options.usdcMint, config.usdcMint),
          "usdc mint"
        ),
        x402RoundBase: bn(
          requireValue(
            optionOrConfig(options.x402RoundBase, config.x402RoundBase),
            "Missing --x402-round-base"
          ),
          "x402 round base"
        ),
        x402RoundPerSig: bn(
          requireValue(
            optionOrConfig(options.x402RoundPerSig, config.x402RoundPerSig),
            "Missing --x402-round-per-sig"
          ),
          "x402 round per signature"
        ),
      };
      const treasury = getAssociatedTokenAddressSync(
        args.usdcMint,
        protocolConfig,
        true
      );

      const signature = await (ctx.program.methods as any)
        .initialize(args)
        .accountsPartial({
          protocolConfig,
          registryState,
          registry,
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
        registry: registry.toBase58(),
        treasury: treasury.toBase58(),
      });
    });
}

function parsePubkeyOption(
  value: string | undefined,
  label: string
): PublicKey {
  return parsePublicKey(
    requireValue(value, `Missing --${label.replace(/\s+/g, "-")}`),
    label
  );
}
