import { Commitment } from "@solana/web3.js";
import fs from "fs";
import path from "path";
import { defaultWalletPath, expandHome, resolveFromCwd } from "./utils";

export type MolphaCluster = "localnet" | "devnet" | "mainnet";

export type CliGlobalOptions = {
  cluster?: MolphaCluster;
  rpc?: string;
  wallet?: string;
  programId?: string;
  commitment?: Commitment;
  idl?: string;
};

export type ResolvedCliOptions = {
  cluster: MolphaCluster;
  rpcUrl: string;
  walletPath: string;
  programId: string;
  commitment: Commitment;
  idlPath: string;
  anchorTomlPath: string;
};

const DEFAULT_RPCS: Record<MolphaCluster, string> = {
  localnet: "http://127.0.0.1:8899",
  devnet: "https://api.devnet.solana.com",
  mainnet: "https://api.mainnet-beta.solana.com",
};

export function resolveCliOptions(options: CliGlobalOptions): ResolvedCliOptions {
  const anchorTomlPath = resolveFromCwd("Anchor.toml");
  const cluster = options.cluster ?? inferCluster(options.rpc);
  const anchorProgramId = readProgramIdFromAnchorToml(anchorTomlPath, cluster);
  const programId = options.programId ?? anchorProgramId;

  if (!programId) {
    throw new Error(`Could not resolve Molpha program id for cluster '${cluster}' from Anchor.toml`);
  }

  return {
    cluster,
    rpcUrl: options.rpc ?? process.env.ANCHOR_PROVIDER_URL ?? DEFAULT_RPCS[cluster],
    walletPath: resolveFromCwd(options.wallet ?? defaultWalletPath()),
    programId,
    commitment: options.commitment ?? "confirmed",
    idlPath: resolveFromCwd(options.idl ?? path.join("target", "idl", "molpha.json")),
    anchorTomlPath,
  };
}

function inferCluster(rpc: string | undefined): MolphaCluster {
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

export function readProgramIdFromAnchorToml(
  anchorTomlPath: string,
  cluster: MolphaCluster
): string | undefined {
  if (!fs.existsSync(anchorTomlPath)) {
    return undefined;
  }

  const source = fs.readFileSync(anchorTomlPath, "utf8");
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

export function ensureIdlPresent(idlPath: string): void {
  if (!fs.existsSync(idlPath)) {
    throw new Error(
      `IDL not found at ${expandHome(idlPath)}. Run 'anchor build' or pass --idl explicitly.`
    );
  }
}
