import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import { u32Le, u64Le } from "./utils";

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const REGISTRY_SEED = Buffer.from("molpha_registry");
/** Permanent owner-keyed node identity and reward account. */
export const NODE_SEED = Buffer.from("molpha_node");
/** `KeyXClaim` seed prefix: `[KEY_X_CLAIM_SEED, x]`, the x coordinate of a node or provider key. */
export const KEY_X_CLAIM_SEED = Buffer.from("molpha_key_x");
/** `Provider` seed prefix: `[PROVIDER_SEED, id_u32_le]`. */
export const PROVIDER_SEED = Buffer.from("molpha_provider");
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

/**
 * The claim on a signing key's x coordinate. Takes the 33-byte compressed key or the 32-byte x
 * itself; a key and its negation share one claim.
 */
export function findKeyXClaimPda(
  programId: PublicKey,
  secp256k1Key: Uint8Array,
): [PublicKey, number] {
  let x: Uint8Array;
  if (secp256k1Key.length === 33) x = secp256k1Key.subarray(1);
  else if (secp256k1Key.length === 32) x = secp256k1Key;
  else throw new Error("secp256k1 key must be 33 (compressed) or 32 (x) bytes");
  return PublicKey.findProgramAddressSync(
    [KEY_X_CLAIM_SEED, Buffer.from(x)],
    programId,
  );
}

/** `Provider` PDA of the provider with stable id `id`; also its registry record's `account`. */
export function findProviderPda(
  programId: PublicKey,
  id: number,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [PROVIDER_SEED, u32Le(id)],
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
