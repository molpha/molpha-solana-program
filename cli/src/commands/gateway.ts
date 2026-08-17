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
      "USDC bond amount (base units); defaults to protocol_config.gateway_bond_min"
    )
    .option("--ip <dotted>", "IPv4 endpoint", "10.0.0.1")
    .option("--port <number>", "TCP port", "8080")
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet"
    )
    .option("--authority-usdc <pubkey>", "gateway authority USDC ATA")
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

      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey
      );
      const authorityUsdc = options.authorityUsdc
        ? parsePublicKey(options.authorityUsdc, "authority USDC")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            gatewayAuthority.publicKey
          );
      const gatewayBond = findGatewayBondAta(
        protocolConfig.usdcMint,
        gatewayPda
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
    .command("claim")
    .description("Claim finalized gateway rewards credited by finalize_round")
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair; defaults to provider wallet"
    )
    .option("--destination <pubkey>", "USDC destination ATA")
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

      const gatewayAuthority = options.gatewayWallet
        ? loadKeypair(options.gatewayWallet)
        : ctx.payer;
      const [gatewayPda] = findGatewayPda(
        ctx.programId,
        gatewayAuthority.publicKey
      );
      const destination = options.destination
        ? parsePublicKey(options.destination, "destination")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            gatewayAuthority.publicKey
          );

      const builder = (ctx.program.methods as any)
        .claimGatewayRewards()
        .accountsPartial({
          authority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          treasury: protocolConfig.treasury,
          destination,
          tokenProgram: TOKEN_PROGRAM_ID,
        });

      if (!gatewayAuthority.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([gatewayAuthority]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        gateway: gatewayPda.toBase58(),
        destination: destination.toBase58(),
      });
    });
}
