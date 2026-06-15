"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerInitCommand = registerInitCommand;
const spl_token_1 = require("@solana/spl-token");
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerInitCommand(program) {
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
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const config = options.config ? (0, utils_1.readJsonFile)(options.config) : {};
        const [protocolConfig] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
        const [registryState] = (0, pdas_1.findRegistryStatePda)(ctx.programId);
        const args = {
            registryCooldownSecs: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.registryCooldownSecs, config.registryCooldownSecs), "Missing --registry-cooldown-secs"), "registry cooldown"),
            previousGraceSecs: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.previousGraceSecs, config.previousGraceSecs), "Missing --previous-grace-secs"), "previous grace"),
            withdrawalCooldownSecs: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.withdrawalCooldownSecs, config.withdrawalCooldownSecs), "Missing --withdrawal-cooldown-secs"), "withdrawal cooldown"),
            maxNodeCount: Number((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.maxNodeCount, config.maxNodeCount), "Missing --max-node-count")),
            redundancyBuffer: Number((0, utils_1.optionOrConfig)(options.redundancyBuffer, config.redundancyBuffer) ?? 2),
            minimumNodeDeposit: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.minimumNodeDeposit, config.minimumNodeDeposit), "Missing --minimum-node-deposit"), "minimum node deposit"),
            rewardPerSignature: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.rewardPerSignature, config.rewardPerSignature), "Missing --reward-per-signature"), "reward per signature"),
            gatewayFee: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.gatewayFee, config.gatewayFee), "Missing --gateway-fee"), "gateway fee"),
            receiptSubmitterBonus: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.receiptSubmitterBonus, config.receiptSubmitterBonus), "Missing --receipt-submitter-bonus"), "receipt submitter bonus"),
            protocolFeeBps: Number((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.protocolFeeBps, config.protocolFeeBps), "Missing --protocol-fee-bps")),
            maxReceiptDelaySlots: (0, utils_1.bn)((0, utils_1.requireValue)((0, utils_1.optionOrConfig)(options.maxReceiptDelaySlots, config.maxReceiptDelaySlots), "Missing --max-receipt-delay-slots"), "max receipt delay"),
            usdcMint: parsePubkeyOption((0, utils_1.optionOrConfig)(options.usdcMint, config.usdcMint), "usdc mint"),
        };
        const treasury = (0, spl_token_1.getAssociatedTokenAddressSync)(args.usdcMint, protocolConfig, true);
        const signature = await ctx.program.methods
            .initialize(args)
            .accountsPartial({
            protocolConfig,
            registryState,
            usdcMint: args.usdcMint,
            treasury,
            authority: ctx.wallet.publicKey,
            tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
            associatedTokenProgram: spl_token_1.ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: web3_js_1.SystemProgram.programId,
        })
            .rpc();
        (0, utils_1.writeJson)({
            signature,
            protocolConfig: protocolConfig.toBase58(),
            registryState: registryState.toBase58(),
            treasury: treasury.toBase58(),
        });
    });
}
function parsePubkeyOption(value, label) {
    return (0, utils_1.parsePublicKey)((0, utils_1.requireValue)(value, `Missing --${label.replace(/\s+/g, "-")}`), label);
}
