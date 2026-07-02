import { BN } from "@coral-xyz/anchor";
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import { Command } from "commander";
import {
  accountExists,
  fetchAccountOrNull,
  finalizeTransitionAccounts,
} from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { deriveNodeId } from "../lib/ids";
import {
  findNodeOperatorPda,
  findNodePda,
  findProtocolConfigPda,
  findRegistryStatePda,
  findVirtualNodePda,
} from "../lib/pdas";
import { buildRegisterNodeArgs } from "../lib/signing/register-node";
import {
  bn,
  hexToBytes,
  loadKeypair,
  parseInteger,
  parsePublicKey,
  requireValue,
  writeJson,
} from "../lib/utils";

export function registerRegistryCommands(program: Command): void {
  const registry = program
    .command("registry")
    .description("Manage registry state");

  registry
    .command("register")
    .description("Register a node")
    .requiredOption(
      "--secp-pubkey <hex>",
      "33-byte compressed secp256k1 pubkey"
    )
    .requiredOption("--registration-salt <hex>", "32-byte registration salt")
    .option(
      "--encryption-pubkey <hex>",
      "33-byte encryption pubkey, defaults to secp pubkey"
    )
    .option("--peer-id-hash <hex>", "32-byte peer id hash")
    .option("--metadata-hash <hex>", "32-byte metadata hash")
    .option("--security-deposit <amount>", "security deposit amount")
    .option(
      "--new-index <number>",
      "explicit new index, defaults to current node count"
    )
    .option(
      "--node-owner-wallet <path>",
      "separate node owner keypair; admin remains provider wallet"
    )
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!protocolConfig || !registryState) {
        throw new Error("Protocol config or registry state is not initialized");
      }

      const nodeOwner = options.nodeOwnerWallet
        ? loadKeypair(options.nodeOwnerWallet)
        : ctx.payer;
      const secpPubkey = hexToBytes(options.secpPubkey, 33, "secp pubkey");
      const registrationSalt = hexToBytes(
        options.registrationSalt,
        32,
        "registration salt"
      );
      const newIndex =
        options.newIndex !== undefined
          ? parseInteger(options.newIndex, "new index")
          : Number(registryState.currentNodeCount);
      const nodeId = deriveNodeId(
        nodeOwner.publicKey,
        secpPubkey,
        registrationSalt
      );
      const [nodeOperator] = findNodeOperatorPda(
        ctx.programId,
        nodeOwner.publicKey
      );
      const [node] = findNodePda(ctx.programId, newIndex);
      const ownerUsdc = options.ownerUsdc
        ? parsePublicKey(options.ownerUsdc, "owner USDC account")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            nodeOwner.publicKey
          );

      const args = buildRegisterNodeArgs({
        newIndex,
        registrationSalt,
        secp256k1PubkeyCompressed: secpPubkey,
        encryptionPubkey: options.encryptionPubkey
          ? hexToBytes(options.encryptionPubkey, 33, "encryption pubkey")
          : undefined,
        peerIdHash: options.peerIdHash
          ? hexToBytes(options.peerIdHash, 32, "peer id hash")
          : undefined,
        metadataHash: options.metadataHash
          ? hexToBytes(options.metadataHash, 32, "metadata hash")
          : undefined,
        securityDeposit: options.securityDeposit
          ? bn(options.securityDeposit, "security deposit")
          : new BN(protocolConfig.minimumNodeDeposit.toString()),
      });

      const builder = (ctx.program.methods as any)
        .registerNode(args, Array.from(nodeId))
        .accountsPartial({
          adminAuthority: ctx.wallet.publicKey,
          nodeOwner: nodeOwner.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          nodeOperator,
          node,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        });

      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([nodeOwner]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.publicKey.toBase58(),
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        nodeOperator: nodeOperator.toBase58(),
        node: node.toBase58(),
      });
    });

  registry
    .command("deactivate")
    .description("Deactivate the signer's node")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      // `removed_operator` is seeded by the signer (the program requires
      // authority == operator.owner), so derive it from the wallet.
      const [removedOperator] = findNodeOperatorPda(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const operatorAccount = await fetchAccountOrNull<any>(
        ctx.program,
        "nodeOperator",
        removedOperator
      );
      if (!operatorAccount) {
        throw new Error(
          `Node operator not found at ${removedOperator.toBase58()}`
        );
      }

      const removedIndex = Number(operatorAccount.currentIndex);
      const [removedNode] = findNodePda(ctx.programId, removedIndex);
      const [virtualNode] = findVirtualNodePda(ctx.programId);
      const lastIndex = Math.max(Number(registryState.currentNodeCount) - 1, 0);

      let lastNode = removedNode;
      let movedOperator = removedOperator;
      if (removedIndex !== lastIndex) {
        lastNode = findNodePda(ctx.programId, lastIndex)[0];
        const lastNodeData = await fetchAccountOrNull<any>(
          ctx.program,
          "node",
          lastNode
        );
        if (!lastNodeData) {
          throw new Error(`Last node not found at ${lastNode.toBase58()}`);
        }
        movedOperator = findNodeOperatorPda(
          ctx.programId,
          lastNodeData.owner
        )[0];
      }

      const signature = await (ctx.program.methods as any)
        .deactivateNode()
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          registryState: registryStatePda,
          removedOperator,
          removedNode,
          virtualNode,
          lastNode,
          movedOperator,
          rentReceiver: ctx.wallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      writeJson({
        signature,
        removedOperator: removedOperator.toBase58(),
        removedNode: removedNode.toBase58(),
      });
    });

  registry
    .command("finalize-transition")
    .description("Finalize an expired registry transition")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      const { virtualNode } = await finalizeTransitionAccounts(
        ctx.program,
        registryState
      );
      if (virtualNode && !(await accountExists(ctx.program, virtualNode))) {
        throw new Error(`Virtual node not found at ${virtualNode.toBase58()}`);
      }

      // The deactivated operator receives the virtual slot's residual rewards.
      let nodeOperator = null;
      if (virtualNode) {
        const virtualNodeData = await fetchAccountOrNull<any>(
          ctx.program,
          "node",
          virtualNode
        );
        if (virtualNodeData) {
          nodeOperator = findNodeOperatorPda(
            ctx.programId,
            virtualNodeData.owner
          )[0];
        }
      }

      const signature = await (ctx.program.methods as any)
        .finalizeRegistryTransition()
        .accountsPartial({
          registryState: registryStatePda,
          virtualNode,
          nodeOperator,
          rentReceiver: ctx.wallet.publicKey,
        })
        .rpc();

      writeJson({
        signature,
        virtualNode: virtualNode?.toBase58() ?? null,
      });
    });
}
