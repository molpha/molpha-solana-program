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
  findGatewayPda,
  findNodePda,
  findProtocolConfigPda,
  findTreasuryAta,
} from "../lib/pdas";
import { parsePublicKey, writeJson } from "../lib/utils";

export function registerRewardsCommands(program: Command): void {
  const rewards = program
    .command("rewards")
    .description("Inspect and claim finalized rewards");

  rewards
    .command("balance")
    .description("Show claimable rewards on a permanent node")
    .option("--owner <pubkey>", "node owner; defaults to wallet")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.owner
        ? parsePublicKey(options.owner, "owner")
        : ctx.wallet.publicKey;
      const [nodePda] = findNodePda(ctx.programId, owner);
      const node = await fetchAccountOrNull<any>(ctx.program, "node", nodePda);
      if (!node) throw new Error(`Node not found at ${nodePda.toBase58()}`);
      writeJson({
        owner: owner.toBase58(),
        node: nodePda.toBase58(),
        status: node.status,
        claimableRewards: node.claimableRewards.toString(),
      });
    });

  rewards
    .command("balance-gateway")
    .description("Show claimable gateway rewards")
    .option("--authority <pubkey>", "gateway authority; defaults to wallet")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const authority = options.authority
        ? parsePublicKey(options.authority, "authority")
        : ctx.wallet.publicKey;
      const [gatewayPda] = findGatewayPda(ctx.programId, authority);
      const gateway = await fetchAccountOrNull<any>(
        ctx.program,
        "gateway",
        gatewayPda,
      );
      if (!gateway)
        throw new Error(`Gateway not found at ${gatewayPda.toBase58()}`);
      writeJson({
        authority: authority.toBase58(),
        gateway: gatewayPda.toBase58(),
        claimableRewards: gateway.claimableRewards.toString(),
      });
    });

  rewards
    .command("claim-node")
    .description("Claim finalized rewards from the signer's permanent node")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda,
      );
      if (!protocolConfig)
        throw new Error("Protocol config is not initialized");
      const [node] = findNodePda(ctx.programId, ctx.wallet.publicKey);
      const destination = getAssociatedTokenAddressSync(
        protocolConfig.usdcMint,
        ctx.wallet.publicKey,
      );
      const signature = await (ctx.program.methods as any)
        .claimNodeRewards()
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          node,
          usdcMint: protocolConfig.usdcMint,
          treasury: findTreasuryAta(ctx.programId, protocolConfig.usdcMint),
          destination,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      writeJson({
        signature,
        node: node.toBase58(),
        destination: destination.toBase58(),
      });
    });
}
