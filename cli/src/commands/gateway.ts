import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { findGatewayPda, findProtocolConfigPda } from "../lib/pdas";
import {
  parseInteger,
  parseIpv4,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

export function registerGatewayCommands(program: Command): void {
  const gateway = program.command("gateway").description("Manage gateways");

  gateway
    .command("register")
    .description("Register a gateway (protocol authority only)")
    .option(
      "--gateway-authority <pubkey>",
      "gateway authority; defaults to provider wallet",
    )
    .option("--ip <dotted>", "IPv4 endpoint", "10.0.0.1")
    .option("--port <number>", "TCP port", "8080")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const gatewayAuthority = options.gatewayAuthority
        ? parsePublicKey(options.gatewayAuthority, "gateway authority")
        : ctx.wallet.publicKey;
      const [gatewayPda] = findGatewayPda(ctx.programId, gatewayAuthority);

      const signature = await (ctx.program.methods as any)
        .registerGateway({
          authority: gatewayAuthority,
          ip: parseIpv4(options.ip, "ip"),
          port: parseInteger(options.port, "port"),
        })
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      writeJson({
        signature,
        gatewayAuthority: gatewayAuthority.toBase58(),
        gateway: gatewayPda.toBase58(),
      });
    });

  gateway
    .command("deactivate")
    .description("Deactivate a gateway (protocol authority only)")
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
      });
    });
}
