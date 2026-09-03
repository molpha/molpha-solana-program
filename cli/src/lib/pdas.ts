import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3";
import { u32Le, u64Le } from "./utils";

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const REGISTRY_SEED = Buffer.from("molpha_registry");
/** Permanent owner-keyed node identity and reward account. */
export const NODE_SEED = Buffer.from("molpha_node");
export const NODE_DEPOSITS_SEED = Buffer.from("molpha_node_deposits");
export const SECP256K1_KEY_CLAIM_SEED = Buffer.from("molpha_secp_key");
export const PLAN_SEED = Buffer.from("molpha_plan");
export const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
export const FEED_SEED = Buffer.from("molpha_feed");
export const DELEGATE_SEED = Buffer.from("molpha_delegate");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const X402_ROUND_SEED = Buffer.from("molpha_x402_round");
export const ROUND_SEED = Buffer.from("molpha_round");

export function findProtocolConfigPda(
  programId: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([PROTOCOL_CONFIG_SEED], programId);
}

/// The protocol treasury: the canonical USDC ATA of the `protocol_config` PDA.
///
/// `ProtocolConfig` does not store this address — it is derived, and the program enforces the
/// derivation through its `associated_token` constraints.
export function findTreasuryAta(
  programId: PublicKey,
  usdcMint: PublicKey
): PublicKey {
  const [protocolConfig] = findProtocolConfigPda(programId);
  return getAssociatedTokenAddressSync(usdcMint, protocolConfig, true);
}

/// Authority of the shared node-deposit vault. A PDA distinct from `protocol_config`, so the
/// signer used for treasury payouts cannot reach node principal.
export function findNodeDepositsPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([NODE_DEPOSITS_SEED], programId);
}

/// The one vault holding every node's security deposit.
export function findNodeDepositVault(
  programId: PublicKey,
  usdcMint: PublicKey
): PublicKey {
  const [authority] = findNodeDepositsPda(programId);
  return getAssociatedTokenAddressSync(usdcMint, authority, true);
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
  sourceId: Uint8Array,
  signaturesRequired: number,
  submitter: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      FEED_SEED,
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      submitter.toBuffer(),
    ],
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

/** Gateway-owned USDC ATA that holds the slashable bond. */
export function findGatewayBondAta(
  usdcMint: PublicKey,
  gatewayPda: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, gatewayPda, true);
}

/**
 * Rail-scoped payer identity — `keccak256(rail_id_be ‖ payer_address_bytes)`. `railId` is the
 * payment-rail registry id (0 = none/subscription, 1 = solana, 2 = base).
 */
export function derivePayerId(
  railId: number,
  addressBytes: Uint8Array | Buffer
): Uint8Array {
  const railIdBe = Buffer.alloc(2);
  railIdBe.writeUInt16BE(railId, 0);
  return keccak_256(Buffer.concat([railIdBe, Buffer.from(addressBytes)]));
}

/** Subscription / delegate round receipt PDA. */
export function findSubscriptionRoundPda(
  programId: PublicKey,
  subscriptionOwner: PublicKey,
  gatewayPda: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  timestamp: number | bigint
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      ROUND_SEED,
      subscriptionOwner.toBuffer(),
      gatewayPda.toBuffer(),
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      u64Le(timestamp),
    ],
    programId
  );
}

/** x402 round receipt PDA. */
export function findX402RoundPda(
  programId: PublicKey,
  payerId: Uint8Array | Buffer,
  gateway: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      X402_ROUND_SEED,
      Buffer.from(payerId),
      gateway.toBuffer(),
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      u64Le(canonicalTimestamp),
    ],
    programId
  );
}
