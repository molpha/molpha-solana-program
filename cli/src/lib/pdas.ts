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
/** `EpochState` seed prefix: `[EPOCH_SEED, epoch_u64_le]`. */
export const EPOCH_SEED = Buffer.from("molpha_epoch");
/** `TicketMarker` seed prefix: `[TICKET_SEED, ticketId]`. */
export const TICKET_SEED = Buffer.from("molpha_ticket");
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

export function findEpochPda(
  programId: PublicKey,
  epoch: number | bigint,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [EPOCH_SEED, u64Le(epoch)],
    programId,
  );
}

/** Marker of a submitted ticket, keyed by round identity (`deriveTicketId`). */
export function findTicketMarkerPda(
  programId: PublicKey,
  ticketId: Uint8Array,
): [PublicKey, number] {
  if (ticketId.length !== 32) throw new Error("ticket id must be 32 bytes");
  return PublicKey.findProgramAddressSync(
    [TICKET_SEED, Buffer.from(ticketId)],
    programId,
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
