import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findGatewayBondAta,
  findGatewayPda,
  findProtocolConfigPda,
} from "../lib/pdas";
import {
  bn,
  loadKeypair,
  parseInteger,
  parseIpv4,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

export function registerGatewayCommands(program: Command): void {
  const gateway = program.command("gateway").description("Manage gateways");

  gateway
    .command("register")
    .description("Bond and register a gateway")
    .option(
      "--bond-amount <amount>",
      "USDC bond amount (base units); defaults to protocol_config.gateway_bond_min",
    )
    .option("--ip <dotted>", "IPv4 endpoint", "10.0.0.1")
    .option("--port <number>", "TCP port", "8080")
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda,
      );
      if (!protocolConfig) {
        throw new Error("Protocol config is not initialized");
      }

      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey,
      );
      const authorityUsdc = getAssociatedTokenAddressSync(
        protocolConfig.usdcMint,
        gatewayAuthority.publicKey,
      );
      const gatewayBond = findGatewayBondAta(
        protocolConfig.usdcMint,
        gatewayPda,
      );
      const bondAmount = options.bondAmount
        ? bn(options.bondAmount, "bond amount")
        : protocolConfig.gatewayBondMin;

      const builder = (ctx.program.methods as any)
        .registerGateway({
          ip: parseIpv4(options.ip, "ip"),
          port: parseInteger(options.port, "port"),
          bondAmount,
        })
        .accountsPartial({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          usdcMint: protocolConfig.usdcMint,
          authorityUsdc,
          gatewayBond,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        });

      if (!gatewayAuthority.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([gatewayAuthority]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        gatewayAuthority: gatewayAuthority.publicKey.toBase58(),
        gateway: gatewayPda.toBase58(),
        gatewayBond: gatewayBond.toBase58(),
        bondAmount: bondAmount.toString(),
      });
    });

  gateway
    .command("deactivate")
    .description(
      "Deactivate a gateway and start its dispute-window exit (protocol authority only)",
    )
    .requiredOption(
      "--gateway-authority <pubkey>",
      "gateway authority to deactivate",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority",
      );
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);
      const signature = await (ctx.program.methods as any)
        .deactivateGateway()
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig,
          gateway: gatewayPda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      const gatewayState = await fetchAccountOrNull<any>(
        ctx.program,
        "gateway",
        gatewayPda,
      );
      writeJson({
        signature,
        gatewayAuthority: gatewayAuthority.toBase58(),
        gateway: gatewayPda.toBase58(),
        deactivatedAt: gatewayState?.deactivatedAt,
        withdrawableSlot: gatewayState?.withdrawableSlot,
      });
    });

  gateway
    .command("withdraw-bond")
    .description("Withdraw a deactivated gateway bond after its dispute window")
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const authority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const config = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfig,
      );
      if (!config) throw new Error("Protocol config is not initialized");

      const [gatewayPda] = findGatewayPda(ctx.programId, authority.publicKey);
      const gatewayBond = findGatewayBondAta(config.usdcMint, gatewayPda);
      const destination = getAssociatedTokenAddressSync(
        config.usdcMint,
        authority.publicKey,
      );
      const builder = (ctx.program.methods as any)
        .withdrawGatewayBond()
        .accountsPartial({
          authority: authority.publicKey,
          protocolConfig,
          gateway: gatewayPda,
          usdcMint: config.usdcMint,
          gatewayBond,
          destination,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        });
      if (!authority.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([authority]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        gatewayAuthority: authority.publicKey.toBase58(),
        gateway: gatewayPda.toBase58(),
        closedGatewayBond: gatewayBond.toBase58(),
        destination: destination.toBase58(),
      });
    });

  gateway
    .command("withdrawal-status")
    .description(
      "Show gateway deactivation state and remaining dispute-window slots",
    )
    .option(
      "--gateway-authority <pubkey>",
      "gateway authority; defaults to provider wallet",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const authority = options.gatewayAuthority
        ? parsePublicKey(options.gatewayAuthority, "gateway authority")
        : ctx.wallet.publicKey;
      const [gatewayPda] = findGatewayPda(ctx.programId, authority);
      const gatewayState = await fetchAccountOrNull<any>(
        ctx.program,
        "gateway",
        gatewayPda,
      );
      if (!gatewayState) {
        throw new Error(`Gateway not found at ${gatewayPda.toBase58()}`);
      }

      const currentSlot = await ctx.connection.getSlot(ctx.options.commitment);
      const deactivatedAt = Number(gatewayState.deactivatedAt.toString());
      const withdrawableSlot = Number(gatewayState.withdrawableSlot.toString());
      const deactivated = deactivatedAt > 0 && withdrawableSlot > 0;
      writeJson({
        gateway: gatewayPda.toBase58(),
        gatewayAuthority: authority.toBase58(),
        deactivatedAt,
        withdrawableSlot,
        currentSlot,
        remainingSlots: deactivated
          ? Math.max(0, withdrawableSlot - currentSlot)
          : null,
        ready: deactivated && currentSlot >= withdrawableSlot,
        slashed: gatewayState.slashed,
      });
    });
}
