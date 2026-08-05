import { PublicKey } from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3";
import { u32Le, u64Le } from "./utils";

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const REGISTRY_SEED = Buffer.from("molpha_registry");
/** Permanent owner-keyed node identity and reward account. */
export const NODE_SEED = Buffer.from("molpha_node");
export const SECP256K1_KEY_CLAIM_SEED = Buffer.from("molpha_secp_key");
export const PLAN_SEED = Buffer.from("molpha_plan");
export const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
export const FEED_SEED = Buffer.from("molpha_feed");
export const DELEGATE_SEED = Buffer.from("molpha_delegate");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const AGENT_ESCROW_SEED = Buffer.from("molpha_agent");
export const AGENT_ROUND_SEED = Buffer.from("molpha_agent_round");
export const ROUND_SEED = Buffer.from("molpha_round");

export function findProtocolConfigPda(
  programId: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([PROTOCOL_CONFIG_SEED], programId);
}

export function findRegistryStatePda(
  programId: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([REGISTRY_STATE_SEED], programId);
}

export function findRegistryPda(
  programId: PublicKey,
  version: number
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, u32Le(version)],
    programId
  );
}

/** Permanent owner-keyed `Node` account. */
export function findNodePda(
  programId: PublicKey,
  owner: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, owner.toBuffer()],
    programId
  );
}

export function findSecp256k1KeyClaimPda(
  programId: PublicKey,
  secp256k1PubkeyCompressed: Uint8Array
): [PublicKey, number] {
  if (secp256k1PubkeyCompressed.length !== 33) {
    throw new Error("secp256k1 public key must be 33 bytes");
  }
  const fingerprint = keccak_256(secp256k1PubkeyCompressed);
  return PublicKey.findProgramAddressSync(
    [SECP256K1_KEY_CLAIM_SEED, Buffer.from(fingerprint)],
    programId
  );
}

export function findPlanPda(
  programId: PublicKey,
  planId: number
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([planId])],
    programId
  );
}

export function findSubscriptionPda(
  programId: PublicKey,
  owner: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, owner.toBuffer()],
    programId
  );
}

export function findFeedPda(
  programId: PublicKey,
  feedId: Uint8Array
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [FEED_SEED, Buffer.from(feedId)],
    programId
  );
}

export function findDelegatePda(
  programId: PublicKey,
  owner: PublicKey,
  delegate: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [DELEGATE_SEED, owner.toBuffer(), delegate.toBuffer()],
    programId
  );
}

export function findGatewayPda(
  programId: PublicKey,
  authority: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  );
}

/** Per-(authority, gateway) x402 escrow PDA. */
export function findAgentEscrowPda(
  programId: PublicKey,
  authority: PublicKey,
  gateway: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [AGENT_ESCROW_SEED, authority.toBuffer(), gateway.toBuffer()],
    programId
  );
}

/** Subscription / delegate round receipt PDA. */
export function findSubscriptionRoundPda(
  programId: PublicKey,
  consumerAuthority: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number,
  timestamp: number | bigint
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      ROUND_SEED,
      consumerAuthority.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired & 0xff]),
      u64Le(timestamp),
    ],
    programId
  );
}

/** Agent (x402) round receipt PDA. */
export function findAgentRoundPda(
  programId: PublicKey,
  authority: PublicKey,
  gateway: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      AGENT_ROUND_SEED,
      authority.toBuffer(),
      gateway.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired & 0xff]),
      u64Le(canonicalTimestamp),
    ],
    programId
  );
}
