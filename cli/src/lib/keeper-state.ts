import fs from "fs";
import os from "os";
import path from "path";
import { PublicKey } from "@solana/web3.js";
import { resolveFromCwd } from "./utils";

const STATE_VERSION = 1;

export type KeeperPersistedState = {
  version: number;
  lastSweepAt: number | null;
};

export function defaultKeeperStatePath(
  programId: PublicKey,
  keeper: PublicKey,
): string {
  const name = `${programId.toBase58()}-${keeper.toBase58()}.json`;
  return path.join(os.homedir(), ".config", "molpha", "keeper", name);
}

export function resolveKeeperStatePath(
  programId: PublicKey,
  keeper: PublicKey,
  override?: string,
): string {
  if (override) {
    return resolveFromCwd(override);
  }
  return defaultKeeperStatePath(programId, keeper);
}

export function loadKeeperState(filePath: string): KeeperPersistedState {
  try {
    const raw = fs.readFileSync(filePath, "utf8");
    const parsed = JSON.parse(raw) as Partial<KeeperPersistedState>;
    if (parsed.version !== STATE_VERSION) {
      return emptyKeeperState();
    }
    const lastSweepAt = parsed.lastSweepAt;
    if (
      lastSweepAt !== null &&
      (typeof lastSweepAt !== "number" ||
        !Number.isFinite(lastSweepAt) ||
        !Number.isInteger(lastSweepAt))
    ) {
      return emptyKeeperState();
    }
    return { version: STATE_VERSION, lastSweepAt: lastSweepAt ?? null };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return emptyKeeperState();
    }
    throw error;
  }
}

export function saveKeeperState(
  filePath: string,
  lastSweepAt: number | null,
): void {
  const dir = path.dirname(filePath);
  fs.mkdirSync(dir, { recursive: true });
  const body = JSON.stringify({ version: STATE_VERSION, lastSweepAt }, null, 2);
  const tmp = `${filePath}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, body, "utf8");
  fs.renameSync(tmp, filePath);
}

function emptyKeeperState(): KeeperPersistedState {
  return { version: STATE_VERSION, lastSweepAt: null };
}
