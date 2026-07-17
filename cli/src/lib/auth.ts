import { createHash } from "crypto";
import { PublicKey } from "@solana/web3.js";
import { u64Le } from "./utils";

const DOMAIN_REQAUTH = Buffer.from("MOLPHA_REQAUTH_V1", "utf8");
const DOMAIN_AGENT_REQAUTH = Buffer.from("MOLPHA_AGENT_REQAUTH_V1", "utf8");

function sha256(parts: Buffer[]): Buffer {
  const hash = createHash("sha256");
  for (const part of parts) {
    hash.update(part);
  }
  return hash.digest();
}

/** `H(RequestAuth)` — domain-separated SHA-256 over borsh `(feed_id, timestamp)`. */
export function hashRequestAuthFields(
  feedId: Uint8Array,
  timestamp: number | bigint
): Buffer {
  return sha256([
    DOMAIN_REQAUTH,
    Buffer.concat([Buffer.from(feedId), u64Le(timestamp)]),
  ]);
}

/**
 * `H(AgentRequestAuth)` — domain-separated SHA-256 over borsh
 * `(agent, gateway, feed_id, canonical_timestamp, amount)`.
 */
export function hashAgentRequestAuthFields(
  agent: PublicKey,
  gateway: PublicKey,
  feedId: Uint8Array,
  canonicalTimestamp: number | bigint,
  amount: number | bigint
): Buffer {
  const body = Buffer.concat([
    agent.toBuffer(),
    gateway.toBuffer(),
    Buffer.from(feedId),
    u64Le(canonicalTimestamp),
    u64Le(amount),
  ]);
  return sha256([DOMAIN_AGENT_REQAUTH, body]);
}
