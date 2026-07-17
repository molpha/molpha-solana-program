import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findNodeOperatorPda,
  findNodePda,
  findProtocolConfigPda,
} from "../lib/pdas";
import { parsePublicKey, writeJson } from "../lib/utils";

export function registerRewardsCommands(program: Command): void {
  const rewards = program
    .command("rewards")
    .description("Claim finalized node rewards");

  rewards
    .command("claim-node")
    .description("Claim finalized node/operator rewards")
    .option("--destination <pubkey>", "USDC destination ATA")
    .option(
      "--node-index <number>",
      "optional active slot index; omit after deactivation"
    )
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

      const [nodeOperator] = findNodeOperatorPda(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const operator = await fetchAccountOrNull<any>(
        ctx.program,
        "nodeOperator",
        nodeOperator
      );
      if (!operator) {
        throw new Error(
          `Node operator not found at ${nodeOperator.toBase58()}`
        );
      }

      let node = null;
      if (options.nodeIndex !== undefined) {
        node = findNodePda(ctx.programId, Number(options.nodeIndex))[0];
      } else if (
        operator.currentIndex !== undefined &&
        Number(operator.currentIndex) !== 0xffffffff
      ) {
        const candidate = findNodePda(
          ctx.programId,
          Number(operator.currentIndex)
        )[0];
        if (await fetchAccountOrNull(ctx.program, "node", candidate)) {
          node = candidate;
        }
      }

      const destination = options.destination
        ? parsePublicKey(options.destination, "destination")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            ctx.wallet.publicKey
          );

      const signature = await (ctx.program.methods as any)
        .claimNodeRewards()
        .accountsPartial({
          owner: ctx.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          nodeOperator,
          node,
          treasury: protocolConfig.treasury,
          destination,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      writeJson({
        signature,
        nodeOperator: nodeOperator.toBase58(),
        node: node?.toBase58() ?? null,
        destination: destination.toBase58(),
      });
    });
}
