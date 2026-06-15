import { BN } from "@coral-xyz/anchor";
import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
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
  findNodePda,
  findProtocolConfigPda,
  findRegistryIndexPda,
  findRegistryStatePda,
  findVirtualRegistryIndexPda,
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
  const registry = program.command("registry").description("Manage registry state");

  registry
    .command("register")
    .description("Register a node")
    .requiredOption("--secp-pubkey <hex>", "33-byte compressed secp256k1 pubkey")
    .requiredOption("--registration-salt <hex>", "32-byte registration salt")
    .option("--encryption-pubkey <hex>", "33-byte encryption pubkey, defaults to secp pubkey")
    .option("--peer-id-hash <hex>", "32-byte peer id hash")
    .option("--metadata-hash <hex>", "32-byte metadata hash")
    .option("--security-deposit <amount>", "security deposit amount")
    .option("--new-index <number>", "explicit new index, defaults to current node count")
    .option("--node-owner-wallet <path>", "separate node owner keypair; admin remains provider wallet")
    .option("--owner-usdc <pubkey>", "owner token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const protocolConfig = await fetchAccountOrNull<any>(ctx.program, "protocolConfig", protocolConfigPda);
      const registryState = await fetchAccountOrNull<any>(ctx.program, "registryState", registryStatePda);
      if (!protocolConfig || !registryState) {
        throw new Error("Protocol config or registry state is not initialized");
      }

      const nodeOwner = options.nodeOwnerWallet ? loadKeypair(options.nodeOwnerWallet) : ctx.payer;
      const secpPubkey = hexToBytes(options.secpPubkey, 33, "secp pubkey");
      const registrationSalt = hexToBytes(options.registrationSalt, 32, "registration salt");
      const newIndex =
        options.newIndex !== undefined
          ? parseInteger(options.newIndex, "new index")
          : Number(registryState.currentNodeCount);
      const nodeId = deriveNodeId(nodeOwner.publicKey, secpPubkey, registrationSalt);
      const [nodeAccount] = findNodePda(ctx.programId, nodeId);
      const [registryIndex] = findRegistryIndexPda(ctx.programId, newIndex);
      const ownerUsdc =
        options.ownerUsdc
          ? parsePublicKey(options.ownerUsdc, "owner USDC account")
          : getAssociatedTokenAddressSync(protocolConfig.usdcMint, nodeOwner.publicKey);

      const args = buildRegisterNodeArgs({
        newIndex,
        registrationSalt,
        secp256k1PubkeyCompressed: secpPubkey,
        encryptionPubkey: options.encryptionPubkey
          ? hexToBytes(options.encryptionPubkey, 33, "encryption pubkey")
          : undefined,
        peerIdHash: options.peerIdHash ? hexToBytes(options.peerIdHash, 32, "peer id hash") : undefined,
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
          nodeAccount,
          registryIndex,
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
        nodeAccount: nodeAccount.toBase58(),
        registryIndex: registryIndex.toBase58(),
      });
    });

  registry
    .command("deactivate")
    .description("Deactivate a node")
    .requiredOption("--node-id <hex>", "32-byte node id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(ctx.program, "registryState", registryStatePda);
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      const nodeId = hexToBytes(options.nodeId, 32, "node id");
      const [removedNode] = findNodePda(ctx.programId, nodeId);
      const removedNodeAccount = await fetchAccountOrNull<any>(ctx.program, "nodeAccount", removedNode);
      if (!removedNodeAccount) {
        throw new Error(`Node not found at ${removedNode.toBase58()}`);
      }

      const removedIndex = Number(removedNodeAccount.currentIndex);
      const [removedIndexAccount] = findRegistryIndexPda(ctx.programId, removedIndex);
      const [virtualRegistryIndex] = findVirtualRegistryIndexPda(ctx.programId);
      const lastIndex = Math.max(Number(registryState.currentNodeCount) - 1, 0);

      let lastIndexAccount = removedIndexAccount;
      let movedNode = removedNode;
      if (removedIndex !== lastIndex) {
        lastIndexAccount = findRegistryIndexPda(ctx.programId, lastIndex)[0];
        const lastIndexData = await fetchAccountOrNull<any>(
          ctx.program,
          "registryIndexAccount",
          lastIndexAccount
        );
        if (!lastIndexData) {
          throw new Error(`Last registry index not found at ${lastIndexAccount.toBase58()}`);
        }
        movedNode = findNodePda(ctx.programId, Uint8Array.from(lastIndexData.nodeId))[0];
      }

      const signature = await (ctx.program.methods as any)
        .deactivateNode({
          removedNodeId: Array.from(nodeId),
        })
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          registryState: registryStatePda,
          removedNode,
          removedIndexAccount,
          virtualRegistryIndex,
          lastIndexAccount,
          movedNode,
          rentReceiver: ctx.wallet.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      writeJson({
        signature,
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        removedNode: removedNode.toBase58(),
      });
    });

  registry
    .command("finalize-transition")
    .description("Finalize an expired registry transition")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(ctx.program, "registryState", registryStatePda);
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

      const { virtualRegistryIndex } = await finalizeTransitionAccounts(
        ctx.program,
        registryState
      );
      if (virtualRegistryIndex && !(await accountExists(ctx.program, virtualRegistryIndex))) {
        throw new Error(`Virtual registry index not found at ${virtualRegistryIndex.toBase58()}`);
      }

      const signature = await (ctx.program.methods as any)
        .finalizeRegistryTransition()
        .accountsPartial({
          registryState: registryStatePda,
          virtualRegistryIndex,
          rentReceiver: ctx.wallet.publicKey,
        })
        .rpc();

      writeJson({
        signature,
        virtualRegistryIndex: virtualRegistryIndex?.toBase58() ?? null,
      });
    });
}
