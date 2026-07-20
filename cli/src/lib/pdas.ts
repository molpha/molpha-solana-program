import { PublicKey } from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3";
import { u32Le, u64Le } from "./utils";

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
/** Slot-keyed registry/workflow account (`Node`). */
export const NODE_SEED = Buffer.from("molpha_node");
/** Owner-keyed management account (`NodeOperator`). */
export const NODE_OPERATOR_SEED = Buffer.from("molpha_node_operator");
export const SECP256K1_KEY_CLAIM_SEED = Buffer.from("molpha_secp_key");
export const PLAN_SEED = Buffer.from("molpha_plan");
export const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
export const FEED_SEED = Buffer.from("molpha_feed");
export const DELEGATE_SEED = Buffer.from("molpha_delegate");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const AGENT_ESCROW_SEED = Buffer.from("molpha_agent");
export const AGENT_ROUND_SEED = Buffer.from("molpha_agent_round");
export const ROUND_SEED = Buffer.from("molpha_round");
export const VIRTUAL_INDEX = 0xffffffff;

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

/** Slot-keyed `Node` account (the per-round workflow account). */
export function findNodePda(
  programId: PublicKey,
  index: number
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([NODE_SEED, u32Le(index)], programId);
}

export function findVirtualNodePda(programId: PublicKey): [PublicKey, number] {
  return findNodePda(programId, VIRTUAL_INDEX);
}

/** Owner-keyed `NodeOperator` account (management/lifecycle). */
export function findNodeOperatorPda(
  programId: PublicKey,
  owner: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [NODE_OPERATOR_SEED, owner.toBuffer()],
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
