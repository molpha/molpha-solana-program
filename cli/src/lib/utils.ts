import { BN } from "@coral-xyz/anchor";
import { Keypair, PublicKey } from "@solana/web3.js";
import fs from "fs";
import os from "os";
import path from "path";

export type JsonValue =
  string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

export function expandHome(input: string): string {
  if (input === "~") {
    return os.homedir();
  }
  if (input.startsWith("~/")) {
    return path.join(os.homedir(), input.slice(2));
  }
  return input;
}

export function resolveFromCwd(input: string): string {
  const expanded = expandHome(input);
  return path.isAbsolute(expanded)
    ? expanded
    : path.resolve(process.cwd(), expanded);
}

export function readUtf8(filePath: string): string {
  return fs.readFileSync(resolveFromCwd(filePath), "utf8");
}

export function readJsonFile<T>(filePath: string): T {
  return JSON.parse(readUtf8(filePath)) as T;
}

export function writeJson(value: unknown): void {
  process.stdout.write(`${JSON.stringify(normalizeForJson(value), null, 2)}\n`);
}

export function normalizeForJson(value: unknown): JsonValue {
  if (value === null || value === undefined) {
    return null;
  }
  if (
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (typeof value === "bigint") {
    return value.toString();
  }
  if (value instanceof PublicKey) {
    return value.toBase58();
  }
  if (BN.isBN(value)) {
    return value.toString(10);
  }
  if (value instanceof Uint8Array || Buffer.isBuffer(value)) {
    return `0x${Buffer.from(value).toString("hex")}`;
  }
  if (Array.isArray(value)) {
    return value.map((item) => normalizeForJson(item));
  }
  if (typeof value === "object") {
    const out: Record<string, JsonValue> = {};
    for (const [key, entry] of Object.entries(
      value as Record<string, unknown>,
    )) {
      out[key] = normalizeForJson(entry);
    }
    return out;
  }
  return String(value);
}

export function loadKeypair(filePath: string): Keypair {
  const secret = readJsonFile<number[]>(filePath);
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

export function parsePublicKey(value: string, label = "public key"): PublicKey {
  try {
    return new PublicKey(value);
  } catch (error) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
}

export function parseInteger(value: string, label: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
  return parsed;
}

export function parseBigIntString(value: string, label: string): bigint {
  try {
    return BigInt(value);
  } catch (error) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
}

export function bn(value: string | number | bigint, label: string): BN {
  try {
    return new BN(typeof value === "string" ? value : value.toString());
  } catch (error) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
}

export function optionalNumber(
  value: string | undefined,
  fallback?: number,
): number | undefined {
  if (value === undefined) {
    return fallback;
  }
  return parseInteger(value, "number");
}

export function hexToBytes(
  value: string,
  expectedLength?: number,
  label = "hex",
): Uint8Array {
  const clean = value.startsWith("0x") ? value.slice(2) : value;
  if (!/^[0-9a-fA-F]*$/.test(clean) || clean.length % 2 !== 0) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
  const bytes = Uint8Array.from(Buffer.from(clean, "hex"));
  if (expectedLength !== undefined && bytes.length !== expectedLength) {
    throw new Error(
      `${label} must be ${expectedLength} bytes, got ${bytes.length}`,
    );
  }
  return bytes;
}

export function maybeHexToBytes(
  value: string | undefined,
  expectedLength?: number,
  label?: string,
): Uint8Array | undefined {
  if (!value) {
    return undefined;
  }
  return hexToBytes(value, expectedLength, label);
}

export function bytesToHex(bytes: Uint8Array): string {
  return `0x${Buffer.from(bytes).toString("hex")}`;
}

export function fixedBytes(
  bytes: Uint8Array,
  length: number,
  label: string,
): Uint8Array {
  if (bytes.length !== length) {
    throw new Error(`${label} must be ${length} bytes, got ${bytes.length}`);
  }
  return bytes;
}

export function readHexFile(
  filePath: string,
  expectedLength?: number,
  label?: string,
): Uint8Array {
  return hexToBytes(readUtf8(filePath).trim(), expectedLength, label);
}

export function parseHexOrFile(
  value: string | undefined,
  filePath: string | undefined,
  expectedLength: number,
  label: string,
): Uint8Array {
  if (value) {
    return hexToBytes(value, expectedLength, label);
  }
  if (filePath) {
    return readHexFile(filePath, expectedLength, label);
  }
  throw new Error(`Missing ${label}`);
}

export function u32Le(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0, 0);
  return out;
}

export function u64Le(value: number | bigint): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(BigInt(value), 0);
  return out;
}

/** Parse dotted IPv4 (`127.0.0.1`) into four octets. */
export function parseIpv4(value: string, label = "ip"): number[] {
  const parts = value.split(".").map((part) => Number(part));
  if (
    parts.length !== 4 ||
    parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)
  ) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
  return parts;
}

/** Parse comma-separated bit indices (`0,1,3`) into a sorted unique list. */
export function parseBitList(value: string, label = "bits"): number[] {
  const bits = value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => parseInteger(part, label));
  for (const bit of bits) {
    if (bit < 0 || bit > 255) {
      throw new Error(`${label} must be in 0..255, got ${bit}`);
    }
  }
  return [...new Set(bits)].sort((a, b) => a - b);
}

export function requireValue<T>(
  value: T | undefined | null,
  message: string,
): T {
  if (value === undefined || value === null) {
    throw new Error(message);
  }
  return value;
}

export function defaultWalletPath(): string {
  return (
    process.env.ANCHOR_WALLET ||
    path.join(os.homedir(), ".config/solana/id.json")
  );
}

export function optionOrConfig<T>(
  optionValue: T | undefined,
  configValue: T | undefined,
): T | undefined {
  return optionValue !== undefined ? optionValue : configValue;
}

export function isDefaultPubkey(value: PublicKey): boolean {
  return value.equals(PublicKey.default);
}
