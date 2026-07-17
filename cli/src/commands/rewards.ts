import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import {
  findGatewayPda,
  findNodeOperatorPda,
  findNodePda,
  findProtocolConfigPda,
} from "../lib/pdas";
import { parsePublicKey, writeJson } from "../lib/utils";

export function registerRewardsCommands(program: Command): void {
  const rewards = program
    .command("rewards")
    .description("Inspect and claim finalized node rewards");

  rewards
    .command("balance")
    .description("Show claimable node rewards on the operator and active slot")
    .option("--owner <pubkey>", "node operator owner; defaults to wallet")
    .option(
      "--node-index <number>",
      "optional active slot index; omit after deactivation"
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.owner
        ? parsePublicKey(options.owner, "owner")
        : ctx.wallet.publicKey;
      const [nodeOperator] = findNodeOperatorPda(ctx.programId, owner);
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

      let nodeAddress: string | null = null;
      let slotClaimable = 0n;
      if (options.nodeIndex !== undefined) {
        const node = findNodePda(ctx.programId, Number(options.nodeIndex))[0];
        const nodeAccount = await fetchAccountOrNull<any>(
          ctx.program,
          "node",
          node
        );
        if (nodeAccount) {
          nodeAddress = node.toBase58();
          slotClaimable = BigInt(nodeAccount.claimableRewards.toString());
        }
      } else if (
        operator.currentIndex !== undefined &&
        Number(operator.currentIndex) !== 0xffffffff
      ) {
        const candidate = findNodePda(
          ctx.programId,
          Number(operator.currentIndex)
        )[0];
        const nodeAccount = await fetchAccountOrNull<any>(
          ctx.program,
          "node",
          candidate
        );
        if (nodeAccount) {
          nodeAddress = candidate.toBase58();
          slotClaimable = BigInt(nodeAccount.claimableRewards.toString());
        }
      }

      const operatorClaimable = BigInt(operator.claimableRewards.toString());
      writeJson({
        owner: owner.toBase58(),
        nodeOperator: nodeOperator.toBase58(),
        node: nodeAddress,
        operatorClaimable: operatorClaimable.toString(),
        slotClaimable: slotClaimable.toString(),
        totalClaimable: (operatorClaimable + slotClaimable).toString(),
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
        gatewayPda
      );
      if (!gateway) {
        throw new Error(`Gateway not found at ${gatewayPda.toBase58()}`);
      }

      writeJson({
        authority: authority.toBase58(),
        gateway: gatewayPda.toBase58(),
        claimableRewards: gateway.claimableRewards.toString(),
      });
    });

  rewards
    .command("claim-node")
    .description("Claim finalized node rewards credited by finalize_round")
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
