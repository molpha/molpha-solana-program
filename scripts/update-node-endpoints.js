#!/usr/bin/env node

/**
 * Sync node endpoint (IP/port) from a generated-nodes manifest to on-chain
 * permanent owner-keyed Node accounts via `update_node_endpoint`.
 *
 * For each node in the manifest, fetches the on-chain endpoint and sends an
 * update transaction when it differs from the manifest values.
 *
 * Prerequisites:
 *   - `anchor build -p molpha` (target/idl/molpha.json)
 *   - Nodes registered on-chain; node owners must sign (keys in manifest)
 *   - Fee payer wallet funded (ANCHOR_WALLET): SOL for transaction fees
 *
 * Usage:
 *   node scripts/update-node-endpoints.js [options]
 *
 * Options:
 *   --input, -i <path>   Manifest JSON (default: ./generated-nodes.json)
 *   --cluster <name>     localnet | devnet | mainnet
 *   --rpc <url>          RPC URL (overrides manifest and cluster default)
 *   --wallet <path>      Fee payer keypair (default: ANCHOR_WALLET)
 *   --dry-run            Compare only; do not send transactions
 */

const fs = require("fs");
const path = require("path");
const anchor = require("@coral-xyz/anchor");
const { Keypair, PublicKey, ComputeBudgetProgram } = require("@solana/web3.js");
const {
  resolveConnectionOptions,
  loadProgram,
  VALID_CLUSTERS,
} = require("./lib/molpha-provider");

const NODE_SEED = Buffer.from("molpha_node");
const DEFAULT_INPUT = path.resolve(process.cwd(), "generated-nodes.json");

function parseArgs(argv) {
  const opts = {
    input: DEFAULT_INPUT,
    cluster: null,
    rpc: null,
    wallet: null,
    dryRun: false,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--input" || arg === "-i") {
      opts.input = path.resolve(argv[++i]);
    } else if (arg === "--cluster" || arg === "-c") {
      opts.cluster = argv[++i];
    } else if (arg === "--rpc") {
      opts.rpc = argv[++i];
    } else if (arg === "--wallet" || arg === "-w") {
      opts.wallet = argv[++i];
    } else if (arg === "--dry-run") {
      opts.dryRun = true;
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        fs.readFileSync(__filename, "utf8").split("\n").slice(0, 25).join("\n")
      );
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (opts.cluster && !VALID_CLUSTERS.includes(opts.cluster)) {
    console.error(`--cluster must be one of: ${VALID_CLUSTERS.join(", ")}`);
    process.exit(1);
  }

  return opts;
}

function parseIpv4(value, label = "ip") {
  const parts = value.split(".").map((part) => Number(part));
  if (
    parts.length !== 4 ||
    parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)
  ) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
  return parts;
}

function formatIpv4(ipBytes) {
  return Array.from(ipBytes).join(".");
}

function findNodePda(programId, nodeOwner) {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, nodeOwner.toBuffer()],
    programId
  )[0];
}

function loadManifest(filepath) {
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(filepath, "utf8"));
  } catch (error) {
    throw new Error(`Failed to read ${filepath}: ${error.message}`);
  }
  if (!Array.isArray(raw.nodes) || raw.nodes.length === 0) {
    throw new Error(
      `Invalid manifest at ${filepath}: expected non-empty "nodes" array`
    );
  }
  return raw;
}

function keypairFromNodeRecord(node) {
  if (!node.solana?.secretKey?.length) {
    throw new Error(
      `Node ${node.index ?? "?"} missing solana.secretKey in manifest`
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(node.solana.secretKey));
}

function endpointsMatch(chainIp, chainPort, manifestIp, manifestPort) {
  const targetIp = parseIpv4(manifestIp);
  const targetPort = Number(manifestPort);
  if (!Number.isInteger(targetPort) || targetPort <= 0 || targetPort > 65535) {
    throw new Error(
      `Invalid port for node: ${manifestPort} (expected 1..65535)`
    );
  }

  const chainIpArr = Array.from(chainIp);
  return (
    chainIpArr.length === 4 &&
    chainIpArr.every((octet, i) => octet === targetIp[i]) &&
    Number(chainPort) === targetPort
  );
}

async function main() {
  const opts = parseArgs(process.argv);
  const manifest = loadManifest(opts.input);

  const connectionOpts = resolveConnectionOptions({
    cluster: opts.cluster ?? manifest.cluster ?? null,
    rpc: opts.rpc ?? manifest.rpcUrl ?? null,
    wallet: opts.wallet ?? manifest.wallet ?? null,
  });

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const programIdOverride =
    manifest.programId ?? connectionOpts.programId ?? null;
  const { program, programId } = loadProgram(provider, programIdOverride);

  console.log(`Cluster: ${connectionOpts.cluster}`);
  console.log(`RPC:     ${connectionOpts.rpcUrl}`);
  console.log(`Program: ${programId.toBase58()}`);
  console.log(`Input:   ${opts.input}`);
  console.log(`Nodes:   ${manifest.nodes.length}`);
  if (opts.dryRun) {
    console.log("Mode:    dry-run (no transactions)\n");
  } else {
    console.log(`Fee payer: ${provider.wallet.publicKey.toBase58()}\n`);
  }

  let checked = 0;
  let skipped = 0;
  let updated = 0;
  let unchanged = 0;

  for (const node of manifest.nodes) {
    const label = `Node ${node.index ?? "?"}`;
    const ownerPk = node.solana?.publicKey;
    if (!ownerPk) {
      console.log(`${label}: skipping — missing solana.publicKey`);
      skipped += 1;
      continue;
    }
    if (node.ip == null || node.port == null) {
      console.log(
        `${label} (${ownerPk}): skipping — missing ip/port in manifest`
      );
      skipped += 1;
      continue;
    }

    const nodeOwner = keypairFromNodeRecord(node);
    if (nodeOwner.publicKey.toBase58() !== ownerPk) {
      throw new Error(
        `${label}: solana.publicKey does not match solana.secretKey`
      );
    }

    const nodePda = findNodePda(programId, nodeOwner.publicKey);
    checked += 1;

    let chainNode;
    try {
      chainNode = await program.account.node.fetch(nodePda);
    } catch {
      console.log(
        `${label} (${ownerPk}): skipping — Node not found at ${nodePda.toBase58()}`
      );
      skipped += 1;
      continue;
    }

    const chainEndpoint = `${formatIpv4(chainNode.ip)}:${chainNode.port}`;
    const targetEndpoint = `${node.ip}:${node.port}`;

    if (endpointsMatch(chainNode.ip, chainNode.port, node.ip, node.port)) {
      console.log(`${label} (${ownerPk}): ok ${chainEndpoint}`);
      unchanged += 1;
      continue;
    }

    console.log(
      `${label} (${ownerPk}): mismatch on-chain ${chainEndpoint} -> manifest ${targetEndpoint}`
    );

    if (!chainNode.status?.active) {
      const status =
        typeof chainNode.status === "object"
          ? Object.keys(chainNode.status)[0] ?? "unknown"
          : String(chainNode.status);
      console.log(`   skipping — node status is ${status}, expected Active`);
      skipped += 1;
      continue;
    }

    const args = {
      ip: parseIpv4(node.ip),
      port: Number(node.port),
    };

    if (opts.dryRun) {
      console.log("   dry-run: would call update_node_endpoint");
      updated += 1;
      continue;
    }

    const signature = await program.methods
      .updateNodeEndpoint(args)
      .accountsPartial({
        nodeOwner: nodeOwner.publicKey,
        node: nodePda,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 200_000 }),
      ])
      .signers([nodeOwner])
      .rpc();

    console.log(`   updated: ${signature}`);
    updated += 1;
  }

  console.log(
    `\nDone. checked=${checked} unchanged=${unchanged} updated=${updated} skipped=${skipped}`
  );
}

main().catch((error) => {
  console.error(error.message ?? error);
  process.exit(1);
});
