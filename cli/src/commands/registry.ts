import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { ComputeBudgetProgram, SystemProgram } from "@solana/web3.js";
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
  findSecp256k1KeyClaimPda,
  findVirtualNodePda,
} from "../lib/pdas";
import { buildRegisterNodeArgs } from "../lib/signing/register-node";
import {
  hexToBytes,
  loadKeypair,
  parseInteger,
  parseIpv4,
  parsePublicKey,
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
    .requiredOption("--pop-sig-r <hex>", "32-byte even-Y Schnorr nonce x")
    .requiredOption("--pop-sig-s <hex>", "32-byte Schnorr response scalar")
    .option("--ip <dotted>", "IPv4 endpoint", "127.0.0.1")
    .option("--port <number>", "TCP port", "8080")
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
      const newIndex = Number(registryState.currentNodeCount);
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
      const [secp256k1KeyClaim] = findSecp256k1KeyClaimPda(
        ctx.programId,
        secpPubkey
      );
      const ownerUsdc = options.ownerUsdc
        ? parsePublicKey(options.ownerUsdc, "owner USDC account")
        : getAssociatedTokenAddressSync(
            protocolConfig.usdcMint,
            nodeOwner.publicKey
          );

      const args = buildRegisterNodeArgs({
        registrationSalt,
        secp256k1PubkeyCompressed: secpPubkey,
        popSigR: hexToBytes(options.popSigR, 32, "PoP signature r"),
        popSigS: hexToBytes(options.popSigS, 32, "PoP signature s"),
        ip: parseIpv4(options.ip, "ip"),
        port: parseInteger(options.port, "port"),
      });

      const builder = (ctx.program.methods as any)
        .registerNode(args)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          nodeOwner: nodeOwner.publicKey,
          protocolConfig: protocolConfigPda,
          registry: registryStatePda,
          nodeOperator,
          node,
          secp256K1KeyClaim: secp256k1KeyClaim,
          usdcMint: protocolConfig.usdcMint,
          ownerUsdc,
          treasury: protocolConfig.treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ]);

      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([nodeOwner]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.publicKey.toBase58(),
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        nodeIndex: newIndex,
        nodeOperator: nodeOperator.toBase58(),
        node: node.toBase58(),
      });
    });

  registry
    .command("update-endpoint")
    .description("Update the signer's node IP and port")
    .requiredOption("--ip <dotted>", "IPv4 endpoint")
    .requiredOption("--port <number>", "TCP port")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [nodeOperator] = findNodeOperatorPda(
        ctx.programId,
        ctx.wallet.publicKey
      );
      const operatorAccount = await fetchAccountOrNull<any>(
        ctx.program,
        "nodeOperator",
        nodeOperator
      );
      if (!operatorAccount) {
        throw new Error(
          `Node operator not found at ${nodeOperator.toBase58()}`
        );
      }

      const args = {
        ip: parseIpv4(options.ip, "ip"),
        port: parseInteger(options.port, "port"),
      };

      const signature = await (ctx.program.methods as any)
        .updateNodeEndpoint(args)
        .accountsPartial({
          nodeOwner: ctx.wallet.publicKey,
          nodeOperator,
        })
        .rpc();

      writeJson({
        signature,
        nodeOperator: nodeOperator.toBase58(),
        ip: options.ip,
        port: args.port,
      });
    });

  registry
    .command("deactivate")
    .description("Deactivate the signer's node")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfigPda] = findProtocolConfigPda(ctx.programId);
      const [registryStatePda] = findRegistryStatePda(ctx.programId);
      const registryState = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }

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
          protocolConfig: protocolConfigPda,
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
