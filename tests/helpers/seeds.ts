import { sha256 } from "@noble/hashes/sha2";

export function toSeed(name: string): Buffer {
  return Buffer.from(sha256(Buffer.from(name, "utf-8")));
}
