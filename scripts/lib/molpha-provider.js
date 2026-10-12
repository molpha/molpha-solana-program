/**
 * Shared Anchor provider / program loading for Molpha CLI scripts.
 */
const fs = require("fs");
const path = require("path");
const os = require("os");
const anchor = require("@coral-xyz/anchor");
const { Program } = require("@coral-xyz/anchor");
const { PublicKey } = require("@solana/web3.js");

const VALID_CLUSTERS = ["localnet", "devnet", "mainnet"];
const DEFAULT_RPCS = {
  localnet: "http://127.0.0.1:8899",
  devnet: "https://api.devnet.solana.com",
  mainnet: "https://api.mainnet-beta.solana.com",
};
const ANCHOR_TOML_PATH = path.resolve(__dirname, "../../Anchor.toml");

function inferClusterFromRpc(rpc) {
  if (!rpc) {
    return null;
  }
  if (rpc.includes("devnet")) {
    return "devnet";
  }
  if (rpc.includes("mainnet")) {
    return "mainnet";
  }
  return "localnet";
}

function readAnchorTomlProviderCluster() {
  if (!fs.existsSync(ANCHOR_TOML_PATH)) {
    return null;
  }
  const source = fs.readFileSync(ANCHOR_TOML_PATH, "utf8");
  let inProvider = false;
  for (const rawLine of source.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === "[provider]") {
      inProvider = true;
      continue;
    }
    if (inProvider && line.match(/^\[/)) {
      break;
    }
    if (!inProvider) {
      continue;
    }
    const match = line.match(/^cluster\s*=\s*"([^"]+)"$/);
    if (match && VALID_CLUSTERS.includes(match[1])) {
      return match[1];
    }
  }
  return null;
}

function readProgramIdFromAnchorToml(cluster) {
  if (!fs.existsSync(ANCHOR_TOML_PATH)) {
    return null;
  }
  const source = fs.readFileSync(ANCHOR_TOML_PATH, "utf8");
  const lines = source.split(/\r?\n/);
  let inTargetSection = false;

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) {
      continue;
    }
    const sectionMatch = line.match(/^\[(.+)\]$/);
    if (sectionMatch) {
      inTargetSection = sectionMatch[1] === `programs.${cluster}`;
      continue;
    }
    if (!inTargetSection) {
      continue;
    }
    const programMatch = line.match(/^molpha\s*=\s*"([^"]+)"$/);
    if (programMatch) {
      return programMatch[1];
    }
  }
  return null;
}

/**
 * Use manifest RPC only when the target cluster matches the manifest cluster.
 * Otherwise `--cluster devnet` would still hit a localnet URL baked into the file.
 */
function resolveManifestRpc(manifest, cluster, explicitRpc) {
  if (explicitRpc) {
    return explicitRpc;
  }
  if (!manifest?.rpcUrl) {
    return null;
  }
  const manifestCluster = manifest.cluster ?? null;
  if (manifestCluster && cluster && manifestCluster !== cluster) {
    return null;
  }
  return manifest.rpcUrl;
}

function resolveConnectionOptions(opts = {}) {
  const envRpc = process.env.ANCHOR_PROVIDER_URL;
  const cluster =
    opts.cluster ??
    inferClusterFromRpc(opts.rpc ?? envRpc) ??
    readAnchorTomlProviderCluster() ??
    "localnet";

  const rpcUrl =
    opts.rpc ??
    (opts.cluster ? DEFAULT_RPCS[cluster] : null) ??
    envRpc ??
    DEFAULT_RPCS[cluster];

  const programIdFromToml = readProgramIdFromAnchorToml(cluster);

  const walletPath =
    opts.wallet ??
    process.env.ANCHOR_WALLET ??
    path.join(os.homedir(), ".config/solana/id.json");

  process.env.ANCHOR_PROVIDER_URL = rpcUrl;
  if (opts.wallet) {
    process.env.ANCHOR_WALLET = path.resolve(opts.wallet);
  } else if (!process.env.ANCHOR_WALLET && fs.existsSync(walletPath)) {
    process.env.ANCHOR_WALLET = walletPath;
  }

  return {
    cluster,
    rpcUrl,
    walletPath: process.env.ANCHOR_WALLET ?? walletPath,
    programId: programIdFromToml,
  };
}

function loadProgram(provider, programIdOverride) {
  const idlPath = path.resolve(__dirname, "../../target/idl/molpha.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error(`Missing IDL at ${idlPath}. Run: anchor build -p molpha`);
  }
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  const programId = new PublicKey(programIdOverride ?? idl.address);
  idl.address = programId.toBase58();
  return { program: new Program(idl, provider), programId, idlPath };
}

function createProvider(connectionOpts) {
  resolveConnectionOptions(connectionOpts);
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  return loadProgram(provider, connectionOpts.programId);
}

module.exports = {
  VALID_CLUSTERS,
  DEFAULT_RPCS,
  resolveManifestRpc,
  resolveConnectionOptions,
  loadProgram,
  createProvider,
};
