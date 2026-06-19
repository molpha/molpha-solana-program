"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerRegistryCommands = registerRegistryCommands;
const anchor_1 = require("@coral-xyz/anchor");
const spl_token_1 = require("@solana/spl-token");
const web3_js_1 = require("@solana/web3.js");
const accounts_1 = require("../lib/accounts");
const client_1 = require("../lib/client");
const ids_1 = require("../lib/ids");
const pdas_1 = require("../lib/pdas");
const register_node_1 = require("../lib/signing/register-node");
const utils_1 = require("../lib/utils");
function registerRegistryCommands(program) {
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
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [protocolConfigPda] = (0, pdas_1.findProtocolConfigPda)(
        ctx.programId
      );
      const [registryStatePda] = (0, pdas_1.findRegistryStatePda)(
        ctx.programId
      );
      const protocolConfig = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "protocolConfig",
        protocolConfigPda
      );
      const registryState = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!protocolConfig || !registryState) {
        throw new Error("Protocol config or registry state is not initialized");
      }
      const nodeOwner = options.nodeOwnerWallet
        ? (0, utils_1.loadKeypair)(options.nodeOwnerWallet)
        : ctx.payer;
      const secpPubkey = (0, utils_1.hexToBytes)(
        options.secpPubkey,
        33,
        "secp pubkey"
      );
      const registrationSalt = (0, utils_1.hexToBytes)(
        options.registrationSalt,
        32,
        "registration salt"
      );
      const newIndex =
        options.newIndex !== undefined
          ? (0, utils_1.parseInteger)(options.newIndex, "new index")
          : Number(registryState.currentNodeCount);
      const nodeId = (0, ids_1.deriveNodeId)(
        nodeOwner.publicKey,
        secpPubkey,
        registrationSalt
      );
      const [nodeAccount] = (0, pdas_1.findNodePda)(ctx.programId, nodeId);
      const [registryIndex] = (0, pdas_1.findRegistryIndexPda)(
        ctx.programId,
        newIndex
      );
      const ownerUsdc = options.ownerUsdc
        ? (0, utils_1.parsePublicKey)(options.ownerUsdc, "owner USDC account")
        : (0, spl_token_1.getAssociatedTokenAddressSync)(
            protocolConfig.usdcMint,
            nodeOwner.publicKey
          );
      const args = (0, register_node_1.buildRegisterNodeArgs)({
        newIndex,
        registrationSalt,
        secp256k1PubkeyCompressed: secpPubkey,
        encryptionPubkey: options.encryptionPubkey
          ? (0, utils_1.hexToBytes)(
              options.encryptionPubkey,
              33,
              "encryption pubkey"
            )
          : undefined,
        peerIdHash: options.peerIdHash
          ? (0, utils_1.hexToBytes)(options.peerIdHash, 32, "peer id hash")
          : undefined,
        metadataHash: options.metadataHash
          ? (0, utils_1.hexToBytes)(options.metadataHash, 32, "metadata hash")
          : undefined,
        securityDeposit: options.securityDeposit
          ? (0, utils_1.bn)(options.securityDeposit, "security deposit")
          : new anchor_1.BN(protocolConfig.minimumNodeDeposit.toString()),
      });
      const builder = ctx.program.methods
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
          systemProgram: web3_js_1.SystemProgram.programId,
          tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
        });
      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([nodeOwner]);
      }
      const signature = await builder.rpc();
      (0, utils_1.writeJson)({
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
    .action(async (options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [registryStatePda] = (0, pdas_1.findRegistryStatePda)(
        ctx.programId
      );
      const registryState = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }
      const nodeId = (0, utils_1.hexToBytes)(options.nodeId, 32, "node id");
      const [removedNode] = (0, pdas_1.findNodePda)(ctx.programId, nodeId);
      const removedNodeAccount = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "nodeAccount",
        removedNode
      );
      if (!removedNodeAccount) {
        throw new Error(`Node not found at ${removedNode.toBase58()}`);
      }
      const removedIndex = Number(removedNodeAccount.currentIndex);
      const [removedIndexAccount] = (0, pdas_1.findRegistryIndexPda)(
        ctx.programId,
        removedIndex
      );
      const [virtualRegistryIndex] = (0, pdas_1.findVirtualRegistryIndexPda)(
        ctx.programId
      );
      const lastIndex = Math.max(Number(registryState.currentNodeCount) - 1, 0);
      let lastIndexAccount = removedIndexAccount;
      let movedNode = removedNode;
      if (removedIndex !== lastIndex) {
        lastIndexAccount = (0, pdas_1.findRegistryIndexPda)(
          ctx.programId,
          lastIndex
        )[0];
        const lastIndexData = await (0, accounts_1.fetchAccountOrNull)(
          ctx.program,
          "registryIndexAccount",
          lastIndexAccount
        );
        if (!lastIndexData) {
          throw new Error(
            `Last registry index not found at ${lastIndexAccount.toBase58()}`
          );
        }
        movedNode = (0, pdas_1.findNodePda)(
          ctx.programId,
          Uint8Array.from(lastIndexData.nodeId)
        )[0];
      }
      const signature = await ctx.program.methods
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
          systemProgram: web3_js_1.SystemProgram.programId,
        })
        .rpc();
      (0, utils_1.writeJson)({
        signature,
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        removedNode: removedNode.toBase58(),
      });
    });
  registry
    .command("finalize-transition")
    .description("Finalize an expired registry transition")
    .action(async (_options, command) => {
      const ctx = await (0, client_1.getCliContext)(command);
      const [registryStatePda] = (0, pdas_1.findRegistryStatePda)(
        ctx.programId
      );
      const registryState = await (0, accounts_1.fetchAccountOrNull)(
        ctx.program,
        "registryState",
        registryStatePda
      );
      if (!registryState) {
        throw new Error("Registry state is not initialized");
      }
      const { virtualRegistryIndex } = await (0,
      accounts_1.finalizeTransitionAccounts)(ctx.program, registryState);
      if (
        virtualRegistryIndex &&
        !(await (0, accounts_1.accountExists)(
          ctx.program,
          virtualRegistryIndex
        ))
      ) {
        throw new Error(
          `Virtual registry index not found at ${virtualRegistryIndex.toBase58()}`
        );
      }
      const signature = await ctx.program.methods
        .finalizeRegistryTransition()
        .accountsPartial({
          registryState: registryStatePda,
          virtualRegistryIndex,
          rentReceiver: ctx.wallet.publicKey,
        })
        .rpc();
      (0, utils_1.writeJson)({
        signature,
        virtualRegistryIndex: virtualRegistryIndex?.toBase58() ?? null,
      });
    });
}
