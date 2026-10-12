#!/usr/bin/env node

/**
 * Register nodes from a generated manifest via `register_node`.
 * Every node that is not already on-chain must have `ip` and `port` set.
 * Registration is refused before any transaction if either is missing.
 *
 * Prerequisites:
 *   - `anchor build` (target/idl/molpha.json)
 *   - Protocol initialized (`yarn molpha init`)
 *   - Admin wallet funded (ANCHOR_WALLET): SOL (for owner funding + fees) + USDC for node deposits
 *   - Manifest produced by generate-nodes.js, with ip and port filled in
 *
 * Usage:
 *   node scripts/register-nodes.js [options]
 *
 * Options:
 *   --input <path>        Manifest JSON (default: ./generated-nodes.json)
 *   --cluster <name>      localnet | devnet | mainnet (default: manifest, then Anchor.toml)
 *   --rpc <url>           RPC URL (overrides manifest and cluster default)
 *   --wallet <path>       Payer/admin keypair (default: ANCHOR_WALLET or manifest wallet)
 *   --count <n>           Register only the first N nodes in the manifest
 */

const path = require("path");
const anchor = require("@coral-xyz/anchor");
const { SystemProgram, ComputeBudgetProgram } = require("@solana/web3.js");
const {
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} = require("@solana/spl-token");
const { findSecp256k1KeyClaimPda, signNodePop } = require("./lib/node-pop");
const {
  VALID_CLUSTERS,
  resolveConnectionOptions,
  resolveManifestRpc,
  loadProgram,
} = require("./lib/molpha-provider");
const {
  DEPOSIT_BUFFER_BPS,
  MAX_NODES,
  findProtocolConfigPda,
  findRegistryStatePda,
  findRegistryPda,
  findNodePda,
  findNodeDepositVault,
  loadManifestFromFile,
  prepareLoadedNodes,
  requireNodeEndpoint,
  isNodeRegisteredOnChain,
  ensureOwnerFunded,
  writeOutput,
  printUsage,
} = require("./lib/node-manifest");

const DEFAULT_INPUT = path.resolve(process.cwd(), "generated-nodes.json");

function parseArgs(argv) {
  const opts = {
    cluster: null,
    rpc: null,
    wallet: null,
    count: null,
    input: DEFAULT_INPUT,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--cluster" || arg === "-c") {
      opts.cluster = argv[++i];
    } else if (arg === "--rpc") {
      opts.rpc = argv[++i];
    } else if (arg === "--wallet" || arg === "-w") {
      opts.wallet = argv[++i];
    } else if (arg === "--count" || arg === "-n") {
      opts.count = parseInt(argv[++i], 10);
    } else if (
      arg === "--input" ||
      arg === "-i" ||
      arg === "--output" ||
      arg === "-o"
    ) {
      opts.input = path.resolve(argv[++i]);
    } else if (arg === "--help" || arg === "-h") {
      printUsage(__filename, 23);
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (opts.count !== null && (!Number.isFinite(opts.count) || opts.count < 1)) {
    console.error("--count must be a positive integer");
    process.exit(1);
  }
  if (opts.cluster && !VALID_CLUSTERS.includes(opts.cluster)) {
    console.error(`--cluster must be one of: ${VALID_CLUSTERS.join(", ")}`);
    process.exit(1);
  }

  return opts;
}

function buildRegisterNodeArgs(item) {
  const endpoint = requireNodeEndpoint(item.record);
  return {
    registrationSalt: Array.from(item.registrationSalt),
    ip: endpoint.ip,
    port: endpoint.port,
    secp256K1PubkeyCompressed: Array.from(item.secpCompressed),
    popSigR: Array.from(item.popSigR),
    popSigS: Array.from(item.popSigS),
  };
}

async function main() {
  const opts = parseArgs(process.argv);
  const manifest = loadManifestFromFile(opts.input);
  const cluster = opts.cluster ?? manifest.cluster ?? null;
  const connection = resolveConnectionOptions({
    cluster,
    rpc: resolveManifestRpc(manifest, cluster, opts.rpc),
    wallet: opts.wallet ?? manifest.wallet ?? null,
  });

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  if (
    manifest.programId &&
    connection.programId &&
    manifest.programId !== connection.programId
  ) {
    console.warn(
      `Manifest program id ${manifest.programId} differs from Anchor.toml ${connection.programId}; using the manifest program id`,
    );
  }
  const { program, programId } = loadProgram(
    provider,
    manifest.programId ?? connection.programId,
  );
  const admin = provider.wallet.payer;

  const protocolConfigPda = findProtocolConfigPda(programId);
  const registryStatePda = findRegistryStatePda(programId);

  let protocolConfig;
  try {
    protocolConfig =
      await program.account.protocolConfig.fetch(protocolConfigPda);
  } catch (error) {
    const detail = error.message ?? String(error);
    if (/fetch failed|ECONNREFUSED|ENOTFOUND/i.test(detail)) {
      throw new Error(
        `Cannot reach RPC at ${provider.connection.rpcEndpoint}. (${detail})`,
      );
    }
    throw new Error(`Protocol not initialized. Run init first. (${detail})`);
  }

  if (admin.publicKey.toString() !== protocolConfig.authority.toString()) {
    throw new Error(
      `ANCHOR_WALLET ${admin.publicKey.toBase58()} is not registry admin ${protocolConfig.authority.toBase58()}`,
    );
  }

  const minDeposit = BigInt(protocolConfig.minimumNodeDeposit.toString());
  const depositPerNode = (minDeposit * BigInt(DEPOSIT_BUFFER_BPS)) / 10000n;
  const usdcMint = protocolConfig.usdcMint;
  const treasury = getAssociatedTokenAddressSync(
    usdcMint,
    protocolConfigPda,
    true,
  );
  const nodeDepositVault = findNodeDepositVault(usdcMint, registryStatePda);

  const nodeItems = prepareLoadedNodes(manifest, programId, opts.count);

  console.log(`Cluster: ${connection.cluster}`);
  console.log(`RPC: ${connection.rpcUrl}`);
  console.log(`Program: ${programId.toBase58()}`);
  console.log(`Admin: ${admin.publicKey.toBase58()}`);
  console.log(`Security deposit per node: ${depositPerNode.toString()}`);
  console.log(`Nodes: ${nodeItems.length}`);
  console.log(`Input: ${opts.input}`);
  console.log("");

  const pending = [];
  const endpointErrors = [];
  for (const item of nodeItems) {
    const nodeRecord = item.record;
    console.log(
      `Node ${nodeRecord.index}: Solana ${nodeRecord.solana.publicKey} | EVM ${nodeRecord.evm.address}`,
    );
    const alreadyOnChain = await isNodeRegisteredOnChain(
      provider.connection,
      item.nodePda,
    );
    if (alreadyOnChain) {
      console.log("   Already registered on-chain, skipping");
      continue;
    }
    try {
      const endpoint = requireNodeEndpoint(nodeRecord);
      console.log(`   Endpoint ${nodeRecord.ip}:${endpoint.port}`);
      pending.push(item);
    } catch (error) {
      endpointErrors.push(error.message);
      console.log(`   ${error.message}`);
    }
  }

  if (endpointErrors.length > 0) {
    throw new Error(
      `Refusing to register. Set ip and port on every unregistered node:\n${endpointErrors.join("\n")}`,
    );
  }

  if (pending.length === 0) {
    console.log("\nNothing to register.");
    return;
  }

  let registeredThisRun = 0;
  for (const item of pending) {
    const nodeRecord = item.record;
    const registryState =
      await program.account.registryState.fetch(registryStatePda);
    const currentVersion = Number(registryState.currentVersion);
    const newRegistryVersion = Number(registryState.nextVersion);
    if (newRegistryVersion <= currentVersion) {
      throw new Error("Registry version overflow");
    }
    const currentRegistryPda = findRegistryPda(programId, currentVersion);
    const newRegistryPda = findRegistryPda(programId, newRegistryVersion);
    const currentRegistry =
      await program.account.registry.fetch(currentRegistryPda);
    const chainIndex = Number(currentRegistry.nodeCount);
    if (chainIndex >= MAX_NODES) {
      throw new Error(
        `Registry full (${chainIndex}/${MAX_NODES}); cannot register more nodes`,
      );
    }

    const nodePda = findNodePda(programId, item.solanaOwner.publicKey);
    const keyClaimPda = findSecp256k1KeyClaimPda(
      programId,
      item.secpCompressed,
    );
    nodeRecord.index = chainIndex;
    nodeRecord.onChain = {
      ...nodeRecord.onChain,
      nodePda: nodePda.toBase58(),
      secp256k1KeyClaimPda: keyClaimPda.toBase58(),
      registryVersion: newRegistryVersion,
      registryPda: newRegistryPda.toBase58(),
      nodeOwner: item.solanaOwner.publicKey.toBase58(),
    };

    const ownerUsdc = await ensureOwnerFunded(
      provider.connection,
      admin,
      item.solanaOwner,
      usdcMint,
      depositPerNode,
    );

    const pop = signNodePop({
      programId,
      nodeId: item.nodeId,
      secp256k1PrivateKey: item.secpPrivateKey,
    });
    item.popSigR = pop.popSigR;
    item.popSigS = pop.popSigS;

    const signature = await program.methods
      .registerNode(buildRegisterNodeArgs(item))
      .accountsPartial({
        authority: admin.publicKey,
        nodeOwner: item.solanaOwner.publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        currentRegistry: currentRegistryPda,
        newRegistry: newRegistryPda,
        node: nodePda,
        secp256K1KeyClaim: keyClaimPda,
        ownerUsdc,
        nodeDepositVault,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 450_000 }),
      ])
      .signers([item.solanaOwner])
      .rpc();

    nodeRecord.registration = {
      signature,
      registeredAt: new Date().toISOString(),
    };
    registeredThisRun += 1;
    console.log(`   Registered at index ${chainIndex}: ${signature}`);
  }

  manifest.cluster = connection.cluster;
  manifest.programId = programId.toBase58();
  manifest.rpcUrl = connection.rpcUrl;
  manifest.wallet = connection.walletPath;
  manifest.registryState = registryStatePda.toBase58();
  manifest.protocolConfig = protocolConfigPda.toBase58();
  manifest.usdcMint = usdcMint.toBase58();
  manifest.treasury = treasury.toBase58();
  manifest.nodeDepositVault = nodeDepositVault.toBase58();
  manifest.securityDepositPerNode = depositPerNode.toString();
  manifest.loadedFromFile = true;
  const updated = nodeItems.map((item) => item.record);
  manifest.nodes = opts.count
    ? updated.concat(manifest.nodes.slice(opts.count))
    : updated;
  manifest.count = manifest.nodes.length;

  writeOutput(opts.input, manifest);
  console.log(
    `\nWrote ${manifest.nodes.length} node(s) to ${opts.input} (mode 600)`,
  );
  console.log(`Registered ${registeredThisRun} node(s) this run`);
}

main().catch((err) => {
  console.error(err.message || err);
  if (err.logs) {
    console.error(err.logs.join("\n"));
  }
  process.exit(1);
});
