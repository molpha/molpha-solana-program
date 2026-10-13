import { keccak_256 } from "@noble/hashes/sha3.js";
import { PublicKey } from "@solana/web3.js";
import {
  evmBitmapBitSet,
  RegistryLike,
  registryNode,
} from "./signing/attestation";

export const DOMAIN_TICKET = Buffer.from("MOLPHA_TICKET_V1");
export const DOMAIN_TICKET_ID = Buffer.from("MOLPHA_TICKET_ID_V1");
export const DOMAIN_DISPATCH = Buffer.from("MOLPHA_DISPATCH_V1");
/** Mirrors `MAX_REGISTRY_NODES`: the roster holds at most this many nodes. */
export const MAX_ROSTER = 256;

function u8(value: number, label: string): Buffer {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new Error(`${label} must fit u8`);
  }
  return Buffer.from([value]);
}

function u32Be(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value >>> 0, 0);
  return out;
}

function u64Be(value: number | bigint): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt(value), 0);
  return out;
}

function bytes32(value: Uint8Array, label: string): Buffer {
  if (value.length !== 32) throw new Error(`${label} must be 32 bytes`);
  return Buffer.from(value);
}

/**
 * Round identity, `keccak(DOMAIN_TICKET_ID ‖ source_id ‖ u8 k ‖ be64 timestamp)`:
 * one ticket per round, whichever aggregate carries it. Mirrors `molpha::state::ticket_id`.
 */
export function deriveTicketId(
  sourceId: Uint8Array,
  signaturesRequired: number,
  timestamp: number | bigint,
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      DOMAIN_TICKET_ID,
      bytes32(sourceId, "source id"),
      u8(signaturesRequired, "signatures required"),
      u64Be(timestamp),
    ]),
  );
}

/** `keccak(DOMAIN_TICKET ‖ m ‖ s ‖ commitment)`. Mirrors `molpha::state::ticket_hash`. */
export function deriveTicketHash(
  messageHash: Uint8Array,
  aggSigS: Uint8Array,
  commitment: Uint8Array,
): Uint8Array {
  if (commitment.length !== 20) throw new Error("commitment must be 20 bytes");
  return keccak_256(
    Buffer.concat([
      DOMAIN_TICKET,
      bytes32(messageHash, "message hash"),
      bytes32(aggSigS, "aggregate s"),
      Buffer.from(commitment),
    ]),
  );
}

/** Whether a ticket hash falls under an epoch's threshold `tau` (big-endian u64 prefix). */
export function isTicket(ticketHash: Uint8Array, tau: bigint): boolean {
  return Buffer.from(ticketHash).readBigUInt64BE(0) < tau;
}

/**
 * The message a gateway authority signs (Ed25519) when it dispatches a round.
 * Mirrors `molpha::state::dispatch_message`.
 */
export function deriveDispatchMessage(
  programId: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  registryVersion: number,
  timestamp: number | bigint,
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      DOMAIN_DISPATCH,
      programId.toBuffer(),
      bytes32(sourceId, "source id"),
      u8(signaturesRequired, "signatures required"),
      u32Be(registryVersion),
      u64Be(timestamp),
    ]),
  );
}

/**
 * Roster slot hints for a ticket: for each selected registry position in ascending bit order,
 * the node's existing epoch roster slot, or the next free slot for a node the epoch has not seen.
 */
export function rosterSlots(
  epoch: { rosterLen: number; roster: Array<number[] | Uint8Array> },
  registry: RegistryLike,
  selectionBitmap: Uint8Array,
): number[] {
  const roster = epoch.roster
    .slice(0, Number(epoch.rosterLen))
    .map((key) => new PublicKey(Uint8Array.from(key)).toBase58());
  const slots: number[] = [];
  for (let bit = 0; bit < Number(registry.nodeCount); bit += 1) {
    if (!evmBitmapBitSet(selectionBitmap, bit)) continue;
    const node = registryNode(registry, bit).toBase58();
    const existing = roster.indexOf(node);
    if (existing >= 0) {
      slots.push(existing);
    } else if (roster.length >= MAX_ROSTER) {
      slots.push(MAX_ROSTER);
    } else {
      roster.push(node);
      slots.push(roster.length - 1);
    }
  }
  return slots;
}

/** The decoded `EpochState` fields payout helpers read. */
export type EpochCounters = {
  rosterLen: number;
  signed: number[];
  selected: number[];
  sumSigned: number;
  sumSelected: number;
  minSamples: number;
  minAvailabilityBps: number;
  credited: number[];
};

/**
 * Payout weight of a roster slot: its sampled signature count, zeroed when it has at least
 * `minSamples` selections and an inclusion rate below `minAvailabilityBps` of the network rate.
 * Mirrors `EpochState::weight`.
 */
export function epochWeight(epoch: EpochCounters, slot: number): bigint {
  const signed = BigInt(epoch.signed[slot]);
  const selected = BigInt(epoch.selected[slot]);
  const belowFloor =
    selected >= BigInt(epoch.minSamples) &&
    signed * BigInt(epoch.sumSelected) * 10_000n <
      BigInt(epoch.minAvailabilityBps) * BigInt(epoch.sumSigned) * selected;
  return belowFloor ? 0n : signed;
}

export function isSlotCredited(epoch: EpochCounters, slot: number): boolean {
  return (epoch.credited[slot >> 3] & (1 << (slot & 7))) !== 0;
}

/** Roster slots that carry weight and still await `credit_epoch_rewards`. */
export function creditableSlots(epoch: EpochCounters): number[] {
  const slots: number[] = [];
  for (let slot = 0; slot < Number(epoch.rosterLen); slot += 1) {
    if (epochWeight(epoch, slot) > 0n && !isSlotCredited(epoch, slot)) {
      slots.push(slot);
    }
  }
  return slots;
}
