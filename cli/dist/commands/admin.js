"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerAdminCommands = registerAdminCommands;
const spl_token_1 = require("@solana/spl-token");
const accounts_1 = require("../lib/accounts");
const client_1 = require("../lib/client");
const pdas_1 = require("../lib/pdas");
const utils_1 = require("../lib/utils");
function registerAdminCommands(program) {
  const admin = program.command("admin").description("Admin operations");
  admin
    .command("withdraw-subscription-vault")
    .description("Withdraw the full treasury balance")
    .option("--destination <pubkey>", "destination token account")
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
      const destination = options.destination
        ? (0, utils_1.parsePublicKey)(
            options.destination,
            "destination token account"
          )
        : (0, spl_token_1.getAssociatedTokenAddressSync)(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );
      const { value } = await ctx.connection.getTokenAccountBalance(
        protocolConfig.treasury
      );
      const signature = await ctx.program.methods
        .adminWithdrawSubscriptionVault()
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          usdcMint: protocolConfig.usdcMint,
          treasury: protocolConfig.treasury,
          destination,
          authority: ctx.wallet.publicKey,
          tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
        })
        .rpc();
      (0, utils_1.writeJson)({
        signature,
        amount: value.amount,
        destination,
      });
    });
}
