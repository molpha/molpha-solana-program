import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { Command } from "commander";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { deriveNodeId } from "../lib/ids";
import {
  findNodeDepositVault,
  findNodePda,
  findProtocolConfigPda,
  findRegistryPda,
  findRegistryStatePda,
  findSecp256k1KeyClaimPda,
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

function registryNode(registry: any, index: number): PublicKey {
  const value = registry.nodes[index];
  return new PublicKey(Uint8Array.from(value));
}

export function registerRegistryCommands(program: Command): void {
  const registry = program
    .command("registry")
    .description("Manage versioned node registries");

  registry
    .command("register")
    .description(
      "Register a permanent node and create the next registry version",
    )
    .requiredOption(
      "--secp-pubkey <hex>",
      "33-byte compressed secp256k1 pubkey",
    )
    .requiredOption("--registration-salt <hex>", "32-byte registration salt")
    .requiredOption("--pop-sig-r <hex>", "32-byte even-Y Schnorr nonce x")
    .requiredOption("--pop-sig-s <hex>", "32-byte Schnorr response scalar")
    .option("--ip <dotted>", "IPv4 endpoint", "127.0.0.1")
    .option("--port <number>", "TCP port", "8080")
    .option("--node-owner-wallet <path>", "separate node owner keypair")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const config = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfig,
      );
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryState,
      );
      if (!config || !state) throw new Error("Protocol is not initialized");

      const currentVersion = Number(state.currentVersion);
      const newRegistryVersion = currentVersion + 1;
      const [currentRegistry] = findRegistryPda(ctx.programId, currentVersion);
      const [newRegistry] = findRegistryPda(ctx.programId, newRegistryVersion);
      const current = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        currentRegistry,
      );
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);

      const nodeOwner = options.nodeOwnerWallet
        ? loadKeypair(options.nodeOwnerWallet)
        : ctx.payer;
      const secpPubkey = hexToBytes(options.secpPubkey, 33, "secp pubkey");
      const registrationSalt = hexToBytes(
        options.registrationSalt,
        32,
        "registration salt",
      );
      const nodeId = deriveNodeId(
        nodeOwner.publicKey,
        secpPubkey,
        registrationSalt,
      );
      const [node] = findNodePda(ctx.programId, nodeOwner.publicKey);
      const [secp256K1KeyClaim] = findSecp256k1KeyClaimPda(
        ctx.programId,
        secpPubkey,
      );
      const ownerUsdc = getAssociatedTokenAddressSync(
        config.usdcMint,
        nodeOwner.publicKey,
      );
      // Security deposits live in one vault under registry_state, not in the treasury.
      const nodeDepositVault = findNodeDepositVault(
        ctx.programId,
        config.usdcMint,
      );

      const args = {
        ...buildRegisterNodeArgs({
          registrationSalt,
          secp256k1PubkeyCompressed: secpPubkey,
          popSigR: hexToBytes(options.popSigR, 32, "PoP signature r"),
          popSigS: hexToBytes(options.popSigS, 32, "PoP signature s"),
          ip: parseIpv4(options.ip, "ip"),
          port: parseInteger(options.port, "port"),
        }),
      };

      const builder = (ctx.program.methods as any)
        .registerNode(args)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          nodeOwner: nodeOwner.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          node,
          secp256K1KeyClaim,
          usdcMint: config.usdcMint,
          ownerUsdc,
          nodeDepositVault,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 450_000 }),
        ]);
      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey))
        builder.signers([nodeOwner]);
      const signature = await builder.rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.publicKey.toBase58(),
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        nodeIndex: Number(current.nodeCount),
        node: node.toBase58(),
        registryVersion: newRegistryVersion,
        registry: newRegistry.toBase58(),
      });
    });

  registry
    .command("update-endpoint")
    .description("Update the signer's active node endpoint")
    .requiredOption("--ip <dotted>", "IPv4 endpoint")
    .requiredOption("--port <number>", "TCP port")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [node] = findNodePda(ctx.programId, ctx.wallet.publicKey);
      const signature = await (ctx.program.methods as any)
        .updateNodeEndpoint({
          ip: parseIpv4(options.ip, "ip"),
          port: parseInteger(options.port, "port"),
        })
        .accountsPartial({ nodeOwner: ctx.wallet.publicKey, node })
        .rpc();
      writeJson({
        signature,
        node: node.toBase58(),
        ip: options.ip,
        port: options.port,
      });
    });

  registry
    .command("deactivate")
    .description(
      "Deactivate a current member and create the next registry version",
    )
    .option(
      "--node-owner <pubkey>",
      "target owner (defaults to signer; protocol authority may target another owner)",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryState,
      );
      if (!state) throw new Error("Registry state is not initialized");
      const currentVersion = Number(state.currentVersion);
      const newRegistryVersion = currentVersion + 1;
      const [currentRegistry] = findRegistryPda(ctx.programId, currentVersion);
      const [newRegistry] = findRegistryPda(ctx.programId, newRegistryVersion);
      const current = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        currentRegistry,
      );
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);
      const nodeOwner = options.nodeOwner
        ? parsePublicKey(options.nodeOwner, "node owner")
        : ctx.wallet.publicKey;
      const [node] = findNodePda(ctx.programId, nodeOwner);
      let removeIndex = -1;
      for (let index = 0; index < Number(current.nodeCount); index++) {
        if (registryNode(current, index).equals(node)) {
          removeIndex = index;
          break;
        }
      }
      if (removeIndex < 0)
        throw new Error("Node is not a current registry member");

      const signature = await (ctx.program.methods as any)
        .deactivateNode({
          nodeOwner,
          removeIndex,
        })
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          node,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.toBase58(),
        node: node.toBase58(),
        removeIndex,
        registryVersion: newRegistryVersion,
      });
    });

  registry
    .command("withdraw-deposit")
    .description("Withdraw a cooled-down node security deposit")
    .option(
      "--node-owner-wallet <path>",
      "node owner keypair; defaults to provider wallet",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.nodeOwnerWallet
        ? loadKeypair(options.nodeOwnerWallet)
        : ctx.payer;
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const config = await fetchAccountOrNull<any>(
        ctx.program,
        "protocolConfig",
        protocolConfig,
      );
      if (!config) throw new Error("Protocol is not initialized");

      const [registryState] = findRegistryStatePda(ctx.programId);
      const [node] = findNodePda(ctx.programId, owner.publicKey);
      const destination = getAssociatedTokenAddressSync(
        config.usdcMint,
        owner.publicKey,
      );
      const builder = (ctx.program.methods as any)
        .withdrawNodeDeposit()
        .accountsPartial({
          owner: owner.publicKey,
          protocolConfig,
          registryState,
          node,
          nodeDepositVault: findNodeDepositVault(
            ctx.programId,
            config.usdcMint,
          ),
          destination,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        });
      if (!owner.publicKey.equals(ctx.wallet.publicKey)) {
        builder.signers([owner]);
      }

      const signature = await builder.rpc();
      writeJson({
        signature,
        owner: owner.publicKey.toBase58(),
        node: node.toBase58(),
        destination: destination.toBase58(),
      });
    });

  registry
    .command("set-buffer")
    .description(
      "Change redundancy buffer by creating the next registry version",
    )
    .requiredOption("--value <number>", "new redundancy buffer")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const value = parseInteger(options.value, "redundancy buffer");
      if (value < 0 || value > 255)
        throw new Error("redundancy buffer must fit u8");
      const [protocolConfig] = findProtocolConfigPda(ctx.programId);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryState,
      );
      if (!state) throw new Error("Registry state is not initialized");
      const currentVersion = Number(state.currentVersion);
      const newRegistryVersion = currentVersion + 1;
      const [currentRegistry] = findRegistryPda(ctx.programId, currentVersion);
      const [newRegistry] = findRegistryPda(ctx.programId, newRegistryVersion);
      const signature = await (ctx.program.methods as any)
        .updateRegistryPolicy(value)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      writeJson({
        signature,
        registryVersion: newRegistryVersion,
        registry: newRegistry.toBase58(),
        redundancyBuffer: value,
      });
    });
}
