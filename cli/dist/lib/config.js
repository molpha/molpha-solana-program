"use strict";
var __importDefault =
  (this && this.__importDefault) ||
  function (mod) {
    return mod && mod.__esModule ? mod : { default: mod };
  };
Object.defineProperty(exports, "__esModule", { value: true });
exports.resolveCliOptions = resolveCliOptions;
exports.readProgramIdFromAnchorToml = readProgramIdFromAnchorToml;
exports.ensureIdlPresent = ensureIdlPresent;
const fs_1 = __importDefault(require("fs"));
const path_1 = __importDefault(require("path"));
const utils_1 = require("./utils");
const DEFAULT_RPCS = {
  localnet: "http://127.0.0.1:8899",
  devnet: "https://api.devnet.solana.com",
  mainnet: "https://api.mainnet-beta.solana.com",
};
function resolveCliOptions(options) {
  const anchorTomlPath = (0, utils_1.resolveFromCwd)("Anchor.toml");
  const cluster = options.cluster ?? inferCluster(options.rpc);
  const anchorProgramId = readProgramIdFromAnchorToml(anchorTomlPath, cluster);
  const programId = options.programId ?? anchorProgramId;
  if (!programId) {
    throw new Error(
      `Could not resolve Molpha program id for cluster '${cluster}' from Anchor.toml`
    );
  }
  return {
    cluster,
    rpcUrl:
      options.rpc ?? process.env.ANCHOR_PROVIDER_URL ?? DEFAULT_RPCS[cluster],
    walletPath: (0, utils_1.resolveFromCwd)(
      options.wallet ?? (0, utils_1.defaultWalletPath)()
    ),
    programId,
    commitment: options.commitment ?? "confirmed",
    idlPath: (0, utils_1.resolveFromCwd)(
      options.idl ?? path_1.default.join("target", "idl", "molpha.json")
    ),
    anchorTomlPath,
  };
}
function inferCluster(rpc) {
  if (!rpc) {
    return "localnet";
  }
  if (rpc.includes("devnet")) {
    return "devnet";
  }
  if (rpc.includes("mainnet")) {
    return "mainnet";
  }
  return "localnet";
}
function readProgramIdFromAnchorToml(anchorTomlPath, cluster) {
  if (!fs_1.default.existsSync(anchorTomlPath)) {
    return undefined;
  }
  const source = fs_1.default.readFileSync(anchorTomlPath, "utf8");
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
  return undefined;
}
function ensureIdlPresent(idlPath) {
  if (!fs_1.default.existsSync(idlPath)) {
    throw new Error(
      `IDL not found at ${(0, utils_1.expandHome)(
        idlPath
      )}. Run 'anchor build' or pass --idl explicitly.`
    );
  }
}
