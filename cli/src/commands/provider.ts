import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { randomBytes } from "crypto";
import { Command } from "commander";
import fs from "fs";
import { fetchAccountOrNull } from "../lib/accounts";
import { CliContext, getCliContext } from "../lib/client";
import {
  findKeyXClaimPda,
  findProtocolConfigPda,
  findProviderPda,
  findRegistryPda,
  findRegistryStatePda,
} from "../lib/pdas";
import {
  fetchRegistry,
  providerRegistrationMembers,
  registryExtendInstructions,
} from "../lib/registry";
import {
  PROVIDER_BIT_BASE,
  nextProviderSlot,
  registryMember,
  registryRecords,
} from "../lib/signing/attestation";
import {
  buildKeyRotationArgs,
  deriveKeyRotationDigest,
} from "../lib/signing/key-rotation";
import { deriveNodeId } from "../lib/ids";
import {
  secp256k1PublicKeyCompressed,
  signNodePop,
} from "../lib/signing/node-pop";
import {
  hexToBytes,
  loadKeypair,
  parseInteger,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

/** A provider's libp2p identity: 32 bytes as hex, or an ed25519 public key in base58. */
function parseP2pKey(value: string): Uint8Array {
  const clean = value.startsWith("0x") ? value.slice(2) : value;
  if (/^[0-9a-fA-F]{64}$/.test(clean)) return hexToBytes(clean, 32, "p2p key");
  return parsePublicKey(value, "p2p key").toBytes();
}

/**
 * The `register_node` argument for a provider. Anchor camel-cases the IDL field `p2p_key` to `p2PKey`
 * (a digit starts a new word); any other spelling serialises as 32 zero bytes, which the program
 * refuses as `InvalidP2pKey`. The proof of possession is the node one, over the node id of the
 * provider authority, the key and the salt (`signNodePop`).
 */
export function registerProviderArgs(a: {
  registrationSalt: Uint8Array;
  p2pKey: Uint8Array;
  secpPubkey: Uint8Array;
  popSigR: Uint8Array;
  popSigS: Uint8Array;
}) {
  return {
    registrationSalt: Array.from(a.registrationSalt),
    endpoint: { provider: { p2PKey: Array.from(a.p2pKey) } },
    secp256K1PubkeyCompressed: Array.from(a.secpPubkey),
    popSigR: Array.from(a.popSigR),
    popSigS: Array.from(a.popSigS),
  };
}

/** The status enum as Anchor decodes it (`{ pending: {} }`) as a plain word. */
function statusName(status: Record<string, unknown>): string {
  return Object.keys(status)[0] ?? "unknown";
}

function formatProvider(address: PublicKey, provider: any) {
  const slot = Number(provider.slot);
  return {
    address: address.toBase58(),
    id: Number(provider.id),
    authority: provider.authority.toBase58(),
    status: statusName(provider.status),
    // The bit it signs with while active; a removed provider keeps the slot it last held.
    slot,
    bit: PROVIDER_BIT_BASE + slot,
    signingKey: {
      x: Buffer.from(provider.x).toString("hex"),
      y: Buffer.from(provider.y).toString("hex"),
    },
    // Anchor camel-cases the IDL field `p2p_key` to `p2PKey` (a digit starts a new word).
    p2pKey: Buffer.from(provider.p2PKey ?? provider.p2pKey).toString("hex"),
    keyRotatedAt: provider.keyRotatedAt.toString(),
  };
}

async function loadProvider(
  ctx: CliContext,
  id: number,
): Promise<{ address: PublicKey; provider: any }> {
  const [address] = findProviderPda(ctx.programId, id);
  const provider = await fetchAccountOrNull<any>(
    ctx.program,
    "provider",
    address,
  );
  if (!provider) throw new Error(`Provider ${id} not found`);
  return { address, provider };
}

async function loadRegistryContext(ctx: CliContext) {
  const [registryState] = findRegistryStatePda(ctx.programId);
  const [protocolConfig] = findProtocolConfigPda(ctx.programId);
  const state = await fetchAccountOrNull<any>(
    ctx.program,
    "registryState",
    registryState,
  );
  if (!state) throw new Error("Registry state is not initialized");
  const currentVersion = Number(state.currentVersion);
  const [currentRegistry] = findRegistryPda(ctx.programId, currentVersion);
  const [newRegistry] = findRegistryPda(ctx.programId, currentVersion + 1);
  const current = await fetchRegistry(ctx.program, currentRegistry);
  if (!current)
    throw new Error(`Current registry ${currentVersion} is missing`);
  return {
    state,
    registryState,
    protocolConfig,
    currentVersion,
    currentRegistry,
    newRegistry,
    current,
  };
}

export function registerProviderCommands(program: Command): void {
  const provider = program
    .command("provider")
    .description(
      "Provider signers: register (with the protocol authority, through register_node), eject, rotate keys. A provider co-signs at bit 64 + slot and counts toward the quorum like a node (at most one per attestation).",
    );

  provider
    .command("pop")
    .description(
      "Sign the proof of possession provider register needs: the node PoP over the node id of the provider authority, the key and a registration salt. Reads the secp256k1 private key from a file; never prints it.",
    )
    .requiredOption(
      "--secp-privkey-file <path>",
      "file holding the 32-byte secp256k1 private key as hex",
    )
    .option(
      "--authority <pubkey>",
      "the provider authority that will register it; defaults to the wallet",
    )
    .option(
      "--registration-salt <hex>",
      "32-byte registration salt; a random one when omitted",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const authority = options.authority
        ? parsePublicKey(options.authority, "authority")
        : ctx.wallet.publicKey;
      const secret = hexToBytes(
        fs.readFileSync(options.secpPrivkeyFile, "utf8").trim(),
        32,
        "secp256k1 private key",
      );
      const registrationSalt = options.registrationSalt
        ? hexToBytes(options.registrationSalt, 32, "registration salt")
        : Uint8Array.from(randomBytes(32));
      const secpPubkey = secp256k1PublicKeyCompressed(secret);
      const pop = signNodePop({
        programId: ctx.programId,
        nodeId: deriveNodeId(authority, secpPubkey, registrationSalt),
        secp256k1PrivateKey: secret,
      });
      writeJson({
        authority: authority.toBase58(),
        registrationSalt: `0x${Buffer.from(registrationSalt).toString("hex")}`,
        secpPubkey: `0x${Buffer.from(pop.secp256k1PubkeyCompressed).toString("hex")}`,
        popSigR: `0x${Buffer.from(pop.popSigR).toString("hex")}`,
        popSigS: `0x${Buffer.from(pop.popSigS).toString("hex")}`,
      });
    });

  provider
    .command("register")
    .description(
      "Register a provider signer through register_node, co-signed by the protocol authority (the wallet). It is in the registry at once, at 64 + the lowest free provider slot. No deposit.",
    )
    .requiredOption(
      "--secp-pubkey <hex>",
      "33-byte compressed secp256k1 signing key",
    )
    .requiredOption(
      "--p2p-key <hex|base58>",
      "ed25519 libp2p identity the gateway dials",
    )
    .requiredOption("--registration-salt <hex>", "32-byte registration salt")
    .requiredOption("--pop-sig-r <hex>", "32-byte even-Y Schnorr nonce x")
    .requiredOption("--pop-sig-s <hex>", "32-byte Schnorr response scalar")
    .option("--authority-wallet <path>", "separate provider authority keypair")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const r = await loadRegistryContext(ctx);
      const id = Number(r.state.nextProviderId);
      const slot = nextProviderSlot(r.current);
      if (slot < 0) throw new Error("All provider slots are taken");
      const authority = options.authorityWallet
        ? loadKeypair(options.authorityWallet)
        : ctx.payer;
      const secpPubkey = hexToBytes(options.secpPubkey, 33, "secp pubkey");
      const [providerAddress] = findProviderPda(ctx.programId, id);
      const [keyXClaim] = findKeyXClaimPda(ctx.programId, secpPubkey);
      const builder = (ctx.program.methods as any)
        .registerNode(
          registerProviderArgs({
            registrationSalt: hexToBytes(
              options.registrationSalt,
              32,
              "registration salt",
            ),
            p2pKey: parseP2pKey(options.p2pKey),
            secpPubkey,
            popSigR: hexToBytes(options.popSigR, 32, "PoP r"),
            popSigS: hexToBytes(options.popSigS, 32, "PoP s"),
          }),
        )
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          nodeOwner: authority.publicKey,
          protocolConfig: r.protocolConfig,
          registryState: r.registryState,
          currentRegistry: r.currentRegistry,
          newRegistry: r.newRegistry,
          node: null,
          provider: providerAddress,
          keyXClaim,
          ownerUsdc: null,
          nodeDepositVault: null,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          // The proof of possession is a Schnorr verification: more than the default 200k units.
          ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
          // The provider table grows by the slots up to this one (slot 0 too, the first time).
          ...(await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            r.currentVersion,
            providerRegistrationMembers(r.current, slot),
          )),
        ]);
      if (!authority.publicKey.equals(ctx.wallet.publicKey))
        builder.signers([authority]);
      const signature = await builder.rpc();
      writeJson({
        signature,
        id,
        slot,
        bit: PROVIDER_BIT_BASE + slot,
        provider: providerAddress.toBase58(),
        authority: authority.publicKey.toBase58(),
        registryVersion: r.currentVersion + 1,
      });
    });

  provider
    .command("eject")
    .description(
      "Eject the provider (its authority only; no one else can remove a provider) through deactivate_node: creates the next registry version without it.",
    )
    .requiredOption("--id <number>", "stable provider id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const id = parseInteger(options.id, "id");
      const { address, provider: account } = await loadProvider(ctx, id);
      const r = await loadRegistryContext(ctx);
      const signature = await (ctx.program.methods as any)
        // `node_owner` is unused on the provider path.
        .deactivateNode(ctx.wallet.publicKey)
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig: r.protocolConfig,
          registryState: r.registryState,
          currentRegistry: r.currentRegistry,
          newRegistry: r.newRegistry,
          node: null,
          provider: address,
          systemProgram: SystemProgram.programId,
        })
        // The handler copies every current record before it drops one.
        .preInstructions(
          await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            r.currentVersion,
            registryRecords(r.current),
          ),
        )
        .rpc();
      writeJson({
        signature,
        id,
        wasActive: statusName(account.status) === "active",
        registryVersion: (
          await fetchAccountOrNull<any>(
            ctx.program,
            "registryState",
            r.registryState,
          )
        )?.currentVersion,
      });
    });

  provider
    .command("rotation-digest")
    .description(
      "Print the digest the provider's current and new signing keys both sign to rotate the key",
    )
    .requiredOption("--id <number>", "stable provider id")
    .requiredOption(
      "--new-secp-pubkey <hex>",
      "33-byte compressed secp256k1 pubkey of the new key",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const id = parseInteger(options.id, "id");
      const [address] = findProviderPda(ctx.programId, id);
      const newKey = hexToBytes(options.newSecpPubkey, 33, "new secp pubkey");
      writeJson({
        provider: address.toBase58(),
        digest: `0x${Buffer.from(
          deriveKeyRotationDigest(ctx.programId, address, newKey),
        ).toString("hex")}`,
      });
    });

  provider
    .command("rotate-key")
    .description(
      "Replace the provider's signing key through rotate_node_key (its authority, and the current key's signature) and create the next registry version",
    )
    .requiredOption("--id <number>", "stable provider id")
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
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const id = parseInteger(options.id, "id");
      const { address } = await loadProvider(ctx, id);
      const r = await loadRegistryContext(ctx);
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
      const [keyXClaim] = findKeyXClaimPda(
        ctx.programId,
        Uint8Array.from(proof.newPublicKeyCompressed),
      );
      const signature = await (ctx.program.methods as any)
        .rotateNodeKey(proof)
        .accountsPartial({
          // The signer is the provider authority.
          nodeOwner: ctx.wallet.publicKey,
          protocolConfig: r.protocolConfig,
          registryState: r.registryState,
          currentRegistry: r.currentRegistry,
          newRegistry: r.newRegistry,
          node: null,
          provider: address,
          keyXClaim,
          systemProgram: SystemProgram.programId,
        })
        .preInstructions([
          // Two signature checks, one of them a proof of possession: close to a registration.
          ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
          ...(await registryExtendInstructions(
            ctx.program,
            ctx.wallet.publicKey,
            r.currentVersion,
            registryRecords(r.current),
          )),
        ])
        .rpc();
      writeJson({
        signature,
        id,
        provider: address.toBase58(),
        registryVersion: r.currentVersion + 1,
      });
    });

  provider
    .command("p2p-key")
    .description(
      "Replace the provider's p2p identity through update_node_endpoint (its authority only); no registry version",
    )
    .requiredOption("--id <number>", "stable provider id")
    .requiredOption("--p2p-key <hex|base58>", "new ed25519 libp2p identity")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const id = parseInteger(options.id, "id");
      const { address } = await loadProvider(ctx, id);
      const signature = await (ctx.program.methods as any)
        // Anchor camel-cases the IDL field `p2p_key` to `p2PKey`; see `registerProviderArgs`.
        .updateNodeEndpoint({
          provider: { p2PKey: Array.from(parseP2pKey(options.p2pKey)) },
        })
        .accountsPartial({
          nodeOwner: ctx.wallet.publicKey,
          node: null,
          provider: address,
        })
        .rpc();
      writeJson({ signature, id });
    });

  provider
    .command("show")
    .description("Fetch a provider and whether the current registry holds it")
    .requiredOption("--id <number>", "stable provider id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const id = parseInteger(options.id, "id");
      const { address, provider: account } = await loadProvider(ctx, id);
      const r = await loadRegistryContext(ctx);
      writeJson({
        ...formatProvider(address, account),
        inCurrentRegistry: inRegistry(r.current, address, account),
        currentRegistryVersion: r.currentVersion,
      });
    });

  provider
    .command("list")
    .description("Every provider registered so far, by id")
    .action(async (_options, command: Command) => {
      const ctx = await getCliContext(command);
      const r = await loadRegistryContext(ctx);
      const providers = [];
      for (let id = 1; id < Number(r.state.nextProviderId); id += 1) {
        const loaded = await loadProvider(ctx, id);
        providers.push({
          ...formatProvider(loaded.address, loaded.provider),
          inCurrentRegistry: inRegistry(
            r.current,
            loaded.address,
            loaded.provider,
          ),
        });
      }
      writeJson({
        registryVersion: r.currentVersion,
        nextProviderId: Number(r.state.nextProviderId),
        providers,
      });
    });
}

/** Whether the registry's provider slot `slot` (bit `64 + slot`) holds this provider's PDA. */
function inRegistry(
  registry: Awaited<ReturnType<typeof fetchRegistry>>,
  address: PublicKey,
  provider: any,
): boolean {
  const slot = Number(provider.slot);
  if (!registry || statusName(provider.status) !== "active") {
    return false;
  }
  try {
    return new PublicKey(
      Uint8Array.from(registryMember(registry, PROVIDER_BIT_BASE + slot).node),
    ).equals(address);
  } catch (_error) {
    return false;
  }
}
