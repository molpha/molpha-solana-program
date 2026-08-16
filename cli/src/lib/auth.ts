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

/** `H(RequestAuth)` — domain-separated SHA-256 over borsh `(source_id, signatures_required, timestamp)`. */
export function hashRequestAuthFields(
  sourceId: Uint8Array,
  signaturesRequired: number,
  timestamp: number | bigint
): Buffer {
  return sha256([
    DOMAIN_REQAUTH,
    Buffer.concat([
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      u64Le(timestamp),
    ]),
  ]);
}

/**
 * `H(AgentRequestAuth)` — domain-separated SHA-256 over borsh
 * `(agent, gateway, source_id, signatures_required, canonical_timestamp, amount)`.
 */
export function hashAgentRequestAuthFields(
  agent: PublicKey,
  gateway: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint,
  amount: number | bigint
): Buffer {
  const body = Buffer.concat([
    agent.toBuffer(),
    gateway.toBuffer(),
    Buffer.from(sourceId),
    Buffer.from([signaturesRequired & 0xff]),
    u64Le(canonicalTimestamp),
    u64Le(amount),
  ]);
  return sha256([DOMAIN_AGENT_REQAUTH, body]);
}
