import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3.js";
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
/** Settlement replay-marker seed prefix: `[ROUND_SEED, gatewayPda, roundId]`. */
export const ROUND_SEED = Buffer.from("molpha_round");
/** Domain tag of `RoundReplayMarker::round_id`. */
export const ROUND_ID_DOMAIN = Buffer.from("MOLPHA_ROUND_ID_V1");
export const PUNISHMENT_CASE_SEED = Buffer.from("molpha_punishment");

export function findProtocolConfigPda(
  programId: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([PROTOCOL_CONFIG_SEED], programId);
}

/// The protocol treasury: the canonical USDC ATA of the `protocol_config` PDA.
///
/// `ProtocolConfig` does not store this address — it is derived, and the program enforces the
/// derivation through its `associated_token` constraints.
export function findTreasuryAta(
  programId: PublicKey,
  usdcMint: PublicKey,
): PublicKey {
  const [protocolConfig] = findProtocolConfigPda(programId);
  return getAssociatedTokenAddressSync(usdcMint, protocolConfig, true);
}

/// The one vault holding every node's security deposit.
export function findNodeDepositVault(
  programId: PublicKey,
  usdcMint: PublicKey,
): PublicKey {
  const [authority] = findRegistryStatePda(programId);
  return getAssociatedTokenAddressSync(usdcMint, authority, true);
}

export function findRegistryStatePda(
  programId: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([REGISTRY_STATE_SEED], programId);
}

export function findRegistryPda(
  programId: PublicKey,
  version: number,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, u32Le(version)],
    programId,
  );
}

/** Permanent owner-keyed `Node` account. */
export function findNodePda(
  programId: PublicKey,
  owner: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, owner.toBuffer()],
    programId,
  );
}

export function findSecp256k1KeyClaimPda(
  programId: PublicKey,
  secp256k1PubkeyCompressed: Uint8Array,
): [PublicKey, number] {
  if (secp256k1PubkeyCompressed.length !== 33) {
    throw new Error("secp256k1 public key must be 33 bytes");
  }
  const fingerprint = keccak_256(secp256k1PubkeyCompressed);
  return PublicKey.findProgramAddressSync(
    [SECP256K1_KEY_CLAIM_SEED, Buffer.from(fingerprint)],
    programId,
  );
}

export function findPlanPda(
  programId: PublicKey,
  planId: number,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([planId])],
    programId,
  );
}

export function findSubscriptionPda(
  programId: PublicKey,
  owner: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, owner.toBuffer()],
    programId,
  );
}

export function findFeedPda(
  programId: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  submitter: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      FEED_SEED,
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired & 0xff]),
      submitter.toBuffer(),
    ],
    programId,
  );
}

export function findDelegatePda(
  programId: PublicKey,
  owner: PublicKey,
  delegate: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [DELEGATE_SEED, owner.toBuffer(), delegate.toBuffer()],
    programId,
  );
}

export function findGatewayPda(
  programId: PublicKey,
  authority: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId,
  );
}

/**
 * Rail-scoped payer identity — `keccak256(rail_id_be ‖ payer_address_bytes)`. `railId` is the
 * payment-rail registry id (0 = none/subscription, 1 = solana, 2 = base).
 */
export function derivePayerId(
  railId: number,
  addressBytes: Uint8Array | Buffer,
): Uint8Array {
  const railIdBe = Buffer.alloc(2);
  railIdBe.writeUInt16BE(railId, 0);
  return keccak_256(Buffer.concat([railIdBe, Buffer.from(addressBytes)]));
}

/** Settlement path tag inside the round id (mirrors `RoundReplayKind`). */
export enum RoundReplayKind {
  Subscription = 0,
  X402 = 1,
}

/**
 * `keccak256(DOMAIN || kind || sourceId || signaturesRequired || canonicalTimestamp_le || initiator)`,
 * matching `RoundReplayMarker::round_id` in the program.
 */
export function deriveRoundId(
  kind: RoundReplayKind,
  sourceId: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint,
  initiator: Uint8Array | Buffer,
): Uint8Array {
  if (sourceId.length !== 32) throw new Error("source id must be 32 bytes");
  if (initiator.length !== 32) throw new Error("initiator must be 32 bytes");
  if (
    !Number.isInteger(signaturesRequired) ||
    signaturesRequired < 0 ||
    signaturesRequired > 255
  ) {
    throw new Error("signatures required must fit u8");
  }
  return keccak_256(
    Buffer.concat([
      ROUND_ID_DOMAIN,
      Buffer.from([kind]),
      Buffer.from(sourceId),
      Buffer.from([signaturesRequired]),
      u64Le(canonicalTimestamp),
      Buffer.from(initiator),
    ]),
  );
}

/** Replay-marker PDA for a round settled through `gatewayPda` under `roundId`. */
export function findRoundReplayPda(
  programId: PublicKey,
  gatewayPda: PublicKey,
  roundId: Uint8Array,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [ROUND_SEED, gatewayPda.toBuffer(), Buffer.from(roundId)],
    programId,
  );
}

/**
 * Subscription-path replay marker. Scoped to the consumer that signed the request (the owner or a
 * delegate), not to the subscription owner.
 */
export function findSubscriptionRoundPda(
  programId: PublicKey,
  consumerAuthority: PublicKey,
  gatewayPda: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint,
): [PublicKey, number] {
  return findRoundReplayPda(
    programId,
    gatewayPda,
    deriveRoundId(
      RoundReplayKind.Subscription,
      sourceId,
      signaturesRequired,
      canonicalTimestamp,
      consumerAuthority.toBuffer(),
    ),
  );
}

/** x402-path replay marker, scoped to the rail-scoped `payerId`. */
export function findX402RoundPda(
  programId: PublicKey,
  payerId: Uint8Array | Buffer,
  gatewayPda: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint,
): [PublicKey, number] {
  return findRoundReplayPda(
    programId,
    gatewayPda,
    deriveRoundId(
      RoundReplayKind.X402,
      sourceId,
      signaturesRequired,
      canonicalTimestamp,
      payerId,
    ),
  );
}

export function findPunishmentCasePda(
  programId: PublicKey,
  evidenceHash: Uint8Array,
): [PublicKey, number] {
  if (evidenceHash.length !== 32) {
    throw new Error("evidence hash must be 32 bytes");
  }
  return PublicKey.findProgramAddressSync(
    [PUNISHMENT_CASE_SEED, Buffer.from(evidenceHash)],
    programId,
  );
}
