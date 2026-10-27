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
import { CliContext, getCliContext } from "../lib/client";
import { deriveNodeId } from "../lib/ids";
import { fetchRegistry, registryExtendInstructions } from "../lib/registry";
import {
  liveSlots,
  nextRegistrySlot,
  registryNode,
  registrySlotOf,
} from "../lib/signing/attestation";
import {
  buildKeyRotationArgs,
  deriveKeyRotationDigest,
} from "../lib/signing/key-rotation";
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

export function registerRegistryCommands(program: Command): void {
  const registry = program
    .command("registry")
    .description("Manage versioned node registries");

  registry
    .command("state")
    .description("Fetch the registry state pointer")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const loaded = await loadRegistryState(ctx);
      const currentVersion = Number(loaded.state.currentVersion);
      const nextVersion = Number(loaded.state.nextVersion);
      writeJson({
        address: loaded.address.toBase58(),
        currentVersion,
        nextVersion,
        currentRegistry: findRegistryPda(
          ctx.programId,
          currentVersion,
        )[0].toBase58(),
        nextRegistry: findRegistryPda(ctx.programId, nextVersion)[0].toBase58(),
      });
    });

  registry
    .command("show")
    .description("Fetch a registry snapshot and its member nodes")
    .option("--version <number>", "registry version; defaults to current")
    .option("--address <pubkey>", "registry account address")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const loaded = await resolveRegistry(ctx, options);
      // `nodeCount` is the number of slots, holes included; a removed node leaves a hole and the
      // others keep their slot, so `index` below is the bit each member signs with.
      const nodeCount = Number(loaded.registry.nodeCount);
      const slots = liveSlots(loaded.registry);
      const addresses: PublicKey[] = slots.map((slot) =>
        registryNode(loaded.registry, slot),
      );
      const nodes = await fetchNodes(ctx, addresses);
      writeJson({
        address: loaded.address.toBase58(),
        version: Number(loaded.registry.version),
        nodeCount,
        liveCount: slots.length,
        redundancyBuffer: Number(loaded.registry.redundancyBuffer),
        graceActiveUntil: amountString(loaded.registry.graceActiveUntil),
        // `signingKey` is the key this version verifies the member with; the node account's own
        // key is its current one, which differs after a rotation.
        nodes: addresses.map((address, position) => ({
          ...formatMember(slots[position], address, nodes[position]),
          signingKey: {
            x: bytesHex(loaded.registry.members[slots[position]].x),
            y: bytesHex(loaded.registry.members[slots[position]].y),
          },
        })),
      });
    });

  registry
    .command("node")
    .description("Fetch a permanent node account")
    .option("--owner <pubkey>", "node owner; defaults to wallet")
    .option("--address <pubkey>", "node account address")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const address = options.address
        ? parsePublicKey(options.address, "node address")
        : findNodePda(
            ctx.programId,
            options.owner
              ? parsePublicKey(options.owner, "node owner")
              : ctx.wallet.publicKey,
          )[0];
      const node = await fetchAccountOrNull<any>(ctx.program, "node", address);
      if (!node) throw new Error(`Node not found at ${address.toBase58()}`);

      const membership = await currentMembership(ctx, address);
      writeJson({
        ...formatNode(address, node),
        currentRegistry: membership,
      });
    });

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
      const current = await fetchRegistry(ctx.program, currentRegistry);
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
          // Past 106 members the next version is allocated ahead of the registration.
          ...(await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            currentVersion,
            Number(current.nodeCount) + 1,
          )),
        ]);
      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey))
        builder.signers([nodeOwner]);
      const signature = await builder.rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.publicKey.toBase58(),
        nodeId: `0x${Buffer.from(nodeId).toString("hex")}`,
        // The lowest hole, else the slot after the last.
        nodeIndex: nextRegistrySlot(current),
        node: node.toBase58(),
        registryVersion: newRegistryVersion,
        registry: newRegistry.toBase58(),
      });
    });

  registry
    .command("rotation-digest")
    .description(
      "Print the digest the current and the new key both sign to rotate a node key",
    )
    .requiredOption(
      "--new-secp-pubkey <hex>",
      "33-byte compressed secp256k1 pubkey of the new key",
    )
    .option("--node-owner <pubkey>", "node owner; defaults to the signer")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const owner = options.nodeOwner
        ? parsePublicKey(options.nodeOwner, "node owner")
        : ctx.wallet.publicKey;
      const [node] = findNodePda(ctx.programId, owner);
      const newKey = hexToBytes(options.newSecpPubkey, 33, "new secp pubkey");
      writeJson({
        node: node.toBase58(),
        digest: `0x${Buffer.from(
          deriveKeyRotationDigest(ctx.programId, node, newKey),
        ).toString("hex")}`,
      });
    });

  registry
    .command("rotate-key")
    .description(
      "Replace the signer's node signing key and create the next registry version",
    )
    .requiredOption(
      "--new-secp-pubkey <hex>",
      "33-byte compressed secp256k1 pubkey of the new key",
    )
    .requiredOption(
      "--old-sig-s <hex>",
      "32-byte s of the current key's signature over the rotation digest",
    )
    .requiredOption(
      "--old-sig-commitment <hex>",
      "20-byte nonce address of the current key's signature",
    )
    .requiredOption("--pop-sig-r <hex>", "32-byte even-Y Schnorr nonce x")
    .requiredOption("--pop-sig-s <hex>", "32-byte Schnorr response scalar")
    .option("--node-owner-wallet <path>", "separate node owner keypair")
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
      const current = await fetchRegistry(ctx.program, currentRegistry);
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);

      const nodeOwner = options.nodeOwnerWallet
        ? loadKeypair(options.nodeOwnerWallet)
        : ctx.payer;
      const [node] = findNodePda(ctx.programId, nodeOwner.publicKey);
      const proof = buildKeyRotationArgs({
        newPublicKeyCompressed: hexToBytes(
          options.newSecpPubkey,
          33,
          "new secp pubkey",
        ),
        oldKeySigS: hexToBytes(options.oldSigS, 32, "old key signature s"),
        oldKeyCommitment: hexToBytes(
          options.oldSigCommitment,
          20,
          "old key signature commitment",
        ),
        popSigR: hexToBytes(options.popSigR, 32, "PoP signature r"),
        popSigS: hexToBytes(options.popSigS, 32, "PoP signature s"),
      });
      const [secp256K1KeyClaim] = findSecp256k1KeyClaimPda(
        ctx.programId,
        Uint8Array.from(proof.newPublicKeyCompressed),
      );

      const builder = (ctx.program.methods as any)
        .rotateNodeKey(proof)
        .accountsPartial({
          nodeOwner: nodeOwner.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          node,
          secp256K1KeyClaim,
          systemProgram: SystemProgram.programId,
        })
        .preInstructions([
          // Two signature checks, one of them a proof of possession: close to a registration.
          ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
          ...(await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            currentVersion,
            Number(current.nodeCount),
          )),
        ]);
      if (!nodeOwner.publicKey.equals(ctx.wallet.publicKey))
        builder.signers([nodeOwner]);
      const signature = await builder.rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.publicKey.toBase58(),
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
      const current = await fetchRegistry(ctx.program, currentRegistry);
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);
      const nodeOwner = options.nodeOwner
        ? parsePublicKey(options.nodeOwner, "node owner")
        : ctx.wallet.publicKey;
      const [node] = findNodePda(ctx.programId, nodeOwner);
      const removeIndex = registrySlotOf(current, node);
      if (removeIndex < 0)
        throw new Error("Node is not a current registry member");

      const signature = await (ctx.program.methods as any)
        .deactivateNode(nodeOwner)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          node,
          systemProgram: SystemProgram.programId,
        })
        // The handler copies every current member before it removes one.
        .preInstructions(
          await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            currentVersion,
            Number(current.nodeCount),
          ),
        )
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
      const current = await fetchRegistry(ctx.program, currentRegistry);
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);
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
        .preInstructions(
          await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            currentVersion,
            Number(current.nodeCount),
          ),
        )
        .rpc();
      writeJson({
        signature,
        registryVersion: newRegistryVersion,
        registry: newRegistry.toBase58(),
        redundancyBuffer: value,
      });
    });

  registry
    .command("reinstate")
    .description(
      "Restore a frozen node to the current registry after its freeze window (or earlier as protocol authority)",
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
      const nodeOwner = options.nodeOwner
        ? parsePublicKey(options.nodeOwner, "node owner")
        : ctx.wallet.publicKey;
      const [node] = findNodePda(ctx.programId, nodeOwner);
      const current = await fetchRegistry(ctx.program, currentRegistry);
      if (!current)
        throw new Error(`Current registry ${currentVersion} is missing`);

      const signature = await (ctx.program.methods as any)
        .reinstateNode()
        .accountsPartial({
          caller: ctx.wallet.publicKey,
          protocolConfig,
          registryState,
          currentRegistry,
          newRegistry,
          node,
          systemProgram: SystemProgram.programId,
        })
        // The node rejoins as one more member.
        .preInstructions(
          await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            currentVersion,
            Number(current.nodeCount) + 1,
          ),
        )
        .rpc();
      writeJson({
        signature,
        nodeOwner: nodeOwner.toBase58(),
        node: node.toBase58(),
        registryVersion: newRegistryVersion,
      });
    });
}

async function loadRegistryState(ctx: CliContext): Promise<{
  address: PublicKey;
  state: any;
}> {
  const [address] = findRegistryStatePda(ctx.programId);
  const state = await fetchAccountOrNull<any>(
    ctx.program,
    "registryState",
    address,
  );
  if (!state) throw new Error("Registry state is not initialized");
  return { address, state };
}

async function resolveRegistry(
  ctx: CliContext,
  options: { version?: string; address?: string },
): Promise<{ address: PublicKey; registry: any }> {
  let address: PublicKey;
  if (options.address) {
    address = parsePublicKey(options.address, "registry address");
  } else if (options.version !== undefined) {
    address = findRegistryPda(
      ctx.programId,
      parseInteger(options.version, "version"),
    )[0];
  } else {
    const loaded = await loadRegistryState(ctx);
    address = findRegistryPda(
      ctx.programId,
      Number(loaded.state.currentVersion),
    )[0];
  }
  const registry = await fetchRegistry(ctx.program, address);
  if (!registry) throw new Error(`Registry not found at ${address.toBase58()}`);
  return { address, registry };
}

async function fetchNodes(
  ctx: CliContext,
  addresses: PublicKey[],
): Promise<Array<any | null>> {
  if (addresses.length === 0) return [];
  return (ctx.program.account as any).node.fetchMultiple(addresses);
}

async function currentMembership(
  ctx: CliContext,
  node: PublicKey,
): Promise<{ version: number; member: boolean; index: number | null } | null> {
  const loaded = await fetchAccountOrNull<any>(
    ctx.program,
    "registryState",
    findRegistryStatePda(ctx.programId)[0],
  );
  if (!loaded) return null;
  const version = Number(loaded.currentVersion);
  const registry = await fetchRegistry(
    ctx.program,
    findRegistryPda(ctx.programId, version)[0],
  );
  if (!registry) return { version, member: false, index: null };
  const index = registrySlotOf(registry, node);
  if (index >= 0) return { version, member: true, index };
  return { version, member: false, index: null };
}

function formatMember(index: number, address: PublicKey, node: any | null) {
  if (!node) {
    return { index, address: address.toBase58(), account: null };
  }
  return { index, ...formatNode(address, node) };
}

function formatNode(address: PublicKey, node: any) {
  return {
    address: address.toBase58(),
    owner: new PublicKey(node.authority).toBase58(),
    status: anchorEnumName(node.status),
    ip: formatIpv4(node.ip),
    port: Number(node.port),
    lockedAmount: amountString(node.lockedAmount),
    claimableRewards: amountString(node.claimableRewards),
    registeredAt: amountString(node.registeredAt),
    withdrawableSlot: amountString(node.withdrawableSlot),
    frozenUntil: amountString(node.frozenUntil),
    secp256k1PubkeyX: bytesHex(node.secp256K1PubkeyX),
    secp256k1PubkeyY: bytesHex(node.secp256K1PubkeyY),
    keyRotatedAt: amountString(node.keyRotatedAt),
  };
}

function anchorEnumName(value: unknown): string {
  if (value && typeof value === "object") {
    const keys = Object.keys(value as object);
    if (keys.length === 1) return keys[0];
  }
  return String(value);
}

function formatIpv4(ip: Iterable<number>): string {
  return [...ip].slice(0, 4).join(".");
}

function amountString(value: { toString(): string } | number | string): string {
  return typeof value === "number" || typeof value === "string"
    ? String(value)
    : value.toString();
}

function bytesHex(value: Iterable<number>): string {
  return `0x${Buffer.from(Uint8Array.from(value)).toString("hex")}`;
}
