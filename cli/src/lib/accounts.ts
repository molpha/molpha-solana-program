import { BN, Program } from "@coral-xyz/anchor";
import { AccountMeta, PublicKey } from "@solana/web3.js";
import { findVirtualRegistryIndexPda } from "./pdas";
import { normalizeForJson } from "./utils";

export type RegistryTransitionKind =
  | "none"
  | "add"
  | "removeTail"
  | "removeSwap";

export type RemoveTransitionInfo = {
  removedOldIndex: number;
  transitionType: "removeTail" | "removeSwap";
  movedOldIndex?: number;
};

export async function fetchAccountOrNull<T = any>(
  program: Program,
  accountName: string,
  address: PublicKey
): Promise<T | null> {
  try {
    return (await (program.account as any)[accountName].fetch(address)) as T;
  } catch (error) {
    return null;
  }
}

export async function accountExists(
  program: Program,
  address: PublicKey
): Promise<boolean> {
  const info = await program.provider.connection.getAccountInfo(address);
  return info !== null;
}

export function registryTransitionKind(value: any): RegistryTransitionKind {
  if (!value || typeof value !== "object") {
    return "none";
  }
  if ("add" in value) {
    return "add";
  }
  if ("removeTail" in value) {
    return "removeTail";
  }
  if ("removeSwap" in value) {
    return "removeSwap";
  }
  return "none";
}

export function removeTransitionInfoFromRegistry(
  registryState: any
): RemoveTransitionInfo | undefined {
  const transitionKind = registryTransitionKind(
    registryState.lastTransitionType
  );
  if (transitionKind !== "removeTail" && transitionKind !== "removeSwap") {
    return undefined;
  }

  const removedOldIndex = Number(registryState.removedOldIndex);
  if (transitionKind === "removeTail") {
    return {
      removedOldIndex,
      transitionType: "removeTail",
    };
  }

  const movedOldIndex = Number(registryState.movedOldIndex);
  return {
    removedOldIndex,
    transitionType: "removeSwap",
    movedOldIndex: movedOldIndex === 0xffffffff ? undefined : movedOldIndex,
  };
}

export async function resolveTransitionForRegistryVersion(
  program: Program,
  registryState: any,
  registryVersion: number
): Promise<{ remainingTransition: RemoveTransitionInfo | undefined }> {
  if (registryVersion === registryState.currentVersion) {
    return { remainingTransition: undefined };
  }

  if (registryVersion !== registryState.previousVersion) {
    throw new Error(
      `Registry version ${registryVersion} is not current (${registryState.currentVersion}) or previous (${registryState.previousVersion})`
    );
  }

  const transitionKind = registryTransitionKind(
    registryState.lastTransitionType
  );
  if (transitionKind === "none" || transitionKind === "add") {
    return { remainingTransition: undefined };
  }

  const remainingTransition = removeTransitionInfoFromRegistry(registryState);
  if (!remainingTransition) {
    throw new Error("Registry state is missing remove-transition metadata");
  }

  return { remainingTransition };
}

export async function finalizeTransitionAccounts(
  program: Program,
  registryState: any
): Promise<{
  virtualRegistryIndex: PublicKey | null;
}> {
  const transitionKind = registryTransitionKind(
    registryState.lastTransitionType
  );
  if (transitionKind === "removeTail" || transitionKind === "removeSwap") {
    const virtualRegistryIndex = findVirtualRegistryIndexPda(
      program.programId
    )[0];
    return {
      virtualRegistryIndex: (await accountExists(program, virtualRegistryIndex))
        ? virtualRegistryIndex
        : null,
    };
  }

  return {
    virtualRegistryIndex: null,
  };
}

export function accountJson(address: PublicKey, data: any): unknown {
  return {
    address: address.toBase58(),
    account: normalizeForJson(data),
  };
}

export function decodeVerifyReturnData(bytes: Uint8Array): {
  raw: string;
  value: string;
  canonicalTimestamp: string;
} {
  if (bytes.length !== 72) {
    throw new Error(`Expected 72 bytes of return data, got ${bytes.length}`);
  }

  const timestamp = Buffer.from(bytes.slice(32, 40)).readBigInt64BE(0);
  return {
    raw: `0x${Buffer.from(bytes).toString("hex")}`,
    value: `0x${Buffer.from(bytes.slice(0, 32)).toString("hex")}`,
    canonicalTimestamp: timestamp.toString(),
  };
}

export function toAccountMetaList(addresses: PublicKey[]): AccountMeta[] {
  return addresses.map((pubkey) => ({
    pubkey,
    isSigner: false,
    isWritable: false,
  }));
}

export function bnToNumber(value: BN | number): number {
  return typeof value === "number" ? value : value.toNumber();
}
