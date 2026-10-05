import { Program } from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import { normalizeForJson } from "./utils";

export async function fetchAccountOrNull<T = any>(
  program: Program,
  accountName: string,
  address: PublicKey,
): Promise<T | null> {
  try {
    return (await (program.account as any)[accountName].fetch(address)) as T;
  } catch (_error) {
    return null;
  }
}

export function accountJson(address: PublicKey, data: any): unknown {
  return { address: address.toBase58(), account: normalizeForJson(data) };
}
