import { BN, Program } from "@coral-xyz/anchor";
import { AccountMeta, PublicKey } from "@solana/web3.js";
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

export async function accountExists(
  program: Program,
  address: PublicKey,
): Promise<boolean> {
  return (await program.provider.connection.getAccountInfo(address)) !== null;
}

export function accountJson(address: PublicKey, data: any): unknown {
  return { address: address.toBase58(), account: normalizeForJson(data) };
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
