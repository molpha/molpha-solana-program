import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findProtocolConfigPda } from "../lib/pdas";
import { parsePublicKey, writeJson } from "../lib/utils";

export function registerAdminCommands(program: Command): void {
  const admin = program.command("admin").description("Admin operations");

  admin
    .command("withdraw-subscription-vault")
    .description("Withdraw the full treasury balance")
    .option("--destination <pubkey>", "destination token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const destination = options.destination
        ? parsePublicKey(options.destination, "destination token account")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );
      const { value } = await ctx.connection.getTokenAccountBalance(
        protocolConfig.treasury
      );

      const signature = await (ctx.program.methods as any)
        .adminWithdrawSubscriptionVault()
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          usdcMint: protocolConfig.usdcMint,
          treasury: protocolConfig.treasury,
          destination,
          authority: ctx.wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      writeJson({
        signature,
        amount: value.amount,
        destination,
      });
    });
}
