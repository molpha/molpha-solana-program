#!/usr/bin/env node

/**
 * Generate N oracle node keypairs (Solana owner + EVM secp256k1) and write a
 * JSON manifest. Does not register on-chain. Each node is written with
 * `ip: null` and `port: null`; set both before running register-nodes.js.
 *
 * Prerequisites:
 *   - `anchor build` (target/idl/molpha.json) so PDAs match the program id
 *
 * Usage:
 *   node scripts/generate-nodes.js [options]
 *
 * Options:
 *   --cluster <name>      localnet | devnet | mainnet (default: Anchor.toml provider or localnet)
 *   --rpc <url>           RPC URL (overrides cluster default)
 *   --wallet <path>       Recorded admin wallet path (default: ANCHOR_WALLET or ~/.config/solana/id.json)
 *   --count <n>           Number of nodes to generate (default: 5)
 *   --start-index <n>     First node index written into the manifest (default: 0)
 *   --output <path>       Output JSON path (default: ./generated-nodes.json)
 *   --force               Overwrite the output file if it already exists
 */

const fs = require("fs");
const path = require("path");
const anchor = require("@coral-xyz/anchor");
const { getAssociatedTokenAddressSync } = require("@solana/spl-token");
const {
  VALID_CLUSTERS,
  resolveConnectionOptions,
  loadProgram,
} = require("./lib/molpha-provider");
const {
  findProtocolConfigPda,
  findRegistryStatePda,
  findNodeDepositVault,
  generateFreshNodes,
  writeOutput,
  printUsage,
  MAX_NODES,
} = require("./lib/node-manifest");

const DEFAULT_COUNT = 5;
const DEFAULT_OUTPUT = path.resolve(process.cwd(), "generated-nodes.json");

function parseArgs(argv) {
  const opts = {
    cluster: null,
    rpc: null,
    wallet: null,
    count: DEFAULT_COUNT,
    startIndex: 0,
    output: DEFAULT_OUTPUT,
    force: false,
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
    } else if (arg === "--start-index") {
      opts.startIndex = parseInt(argv[++i], 10);
    } else if (arg === "--output" || arg === "-o") {
      opts.output = path.resolve(argv[++i]);
    } else if (arg === "--force") {
      opts.force = true;
    } else if (arg === "--help" || arg === "-h") {
      printUsage(__filename, 23);
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (!Number.isFinite(opts.count) || opts.count < 1) {
    console.error("--count must be a positive integer");
    process.exit(1);
  }
  if (!Number.isFinite(opts.startIndex) || opts.startIndex < 0) {
    console.error("--start-index must be a non-negative integer");
    process.exit(1);
  }
  if (opts.startIndex + opts.count > MAX_NODES) {
    console.error(
      `Cannot generate ${opts.count} nodes from index ${opts.startIndex}: max is ${MAX_NODES}`,
    );
    process.exit(1);
  }
  if (opts.cluster && !VALID_CLUSTERS.includes(opts.cluster)) {
    console.error(`--cluster must be one of: ${VALID_CLUSTERS.join(", ")}`);
    process.exit(1);
  }

  return opts;
}

async function main() {
  const opts = parseArgs(process.argv);
  if (fs.existsSync(opts.output) && !opts.force) {
    throw new Error(
      `Output already exists: ${opts.output}. Pass --force to overwrite.`,
    );
  }

  const connection = resolveConnectionOptions(opts);
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const { program, programId } = loadProgram(provider, connection.programId);

  const protocolConfigPda = findProtocolConfigPda(programId);
  const registryStatePda = findRegistryStatePda(programId);
  const nodeItems = generateFreshNodes(programId, opts.startIndex, opts.count);

  let usdcMint = null;
  let treasury = null;
  let nodeDepositVault = null;
  try {
    const config =
      await program.account.protocolConfig.fetch(protocolConfigPda);
    usdcMint = config.usdcMint.toBase58();
    treasury = getAssociatedTokenAddressSync(
      config.usdcMint,
      protocolConfigPda,
      true,
    ).toBase58();
    nodeDepositVault = findNodeDepositVault(
      config.usdcMint,
      registryStatePda,
    ).toBase58();
  } catch {
    // Protocol may not be initialized yet. Keys are still valid.
  }

  const manifest = {
    generatedAt: new Date().toISOString(),
    cluster: connection.cluster,
    programId: programId.toBase58(),
    rpcUrl: connection.rpcUrl,
    wallet: connection.walletPath,
    registryState: registryStatePda.toBase58(),
    protocolConfig: protocolConfigPda.toBase58(),
    usdcMint,
    treasury,
    nodeDepositVault,
    startIndex: opts.startIndex,
    count: nodeItems.length,
    securityDepositPerNode: null,
    loadedFromFile: false,
    nodes: nodeItems.map((item) => item.record),
  };

  writeOutput(opts.output, manifest);

  console.log(`Cluster: ${connection.cluster}`);
  console.log(`Program: ${programId.toBase58()}`);
  console.log(`Nodes: ${nodeItems.length}`);
  console.log(`Output: ${opts.output}`);
  console.log(
    "\nEach node has ip and port set to null. Fill both before register-nodes.js.",
  );
  console.log("Keep this file secret — it contains private keys.");
}

main().catch((err) => {
  console.error(err.message || err);
  process.exit(1);
});
