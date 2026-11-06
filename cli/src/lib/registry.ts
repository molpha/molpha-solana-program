import { Program } from "@coral-xyz/anchor";
import {
  PublicKey,
  SystemProgram,
  TransactionInstruction,
} from "@solana/web3.js";
import { findRegistryPda, findRegistryStatePda } from "./pdas";
import {
  RegistryLike,
  RegistryMemberLike,
  registryRecords,
} from "./signing/attestation";

/** Bytes before a registry's member table: the Anchor discriminator and the header. */
export const REGISTRY_HEADER_LEN = 32;
/** One member record: node account, then the signing key's `x` and `y`. */
export const REGISTRY_MEMBER_LEN = 96;
/**
 * Width of a signer bitmap, and so the most records a registry holds: 64 node slots and 192
 * provider slots.
 */
export const MAX_REGISTRY_NODES = 256;
/** Bytes an account can grow by within one top-level instruction. */
export const MAX_GROWTH_PER_INSTRUCTION = 10_240;

/** Size of a registry account with room for `members` members. Mirrors `Registry::space`. */
export function registrySpace(members: number): number {
  return REGISTRY_HEADER_LEN + members * REGISTRY_MEMBER_LEN;
}

/**
 * The two member tables of a registry account, read from its raw data.
 *
 * A registry is a fixed header followed by the `nodeCount` node records and then the
 * `providerCount` provider records, and the account is exactly as long as they need. The IDL
 * describes the header only, so the tables are decoded here.
 */
export function decodeRegistryMembers(
  data: Uint8Array,
  nodeCount: number,
  providerCount = 0,
): { members: RegistryMemberLike[]; providers: RegistryMemberLike[] } {
  const table = data.length - REGISTRY_HEADER_LEN;
  if (table < 0 || table % REGISTRY_MEMBER_LEN !== 0) {
    throw new Error(
      `registry account is ${data.length} bytes, not a header plus whole member records`,
    );
  }
  if (nodeCount + providerCount > table / REGISTRY_MEMBER_LEN) {
    throw new Error(
      `registry claims ${nodeCount} node and ${providerCount} provider slots but holds ${table / REGISTRY_MEMBER_LEN} records`,
    );
  }
  const record = (index: number): RegistryMemberLike => {
    const offset = REGISTRY_HEADER_LEN + index * REGISTRY_MEMBER_LEN;
    return {
      node: Uint8Array.from(data.subarray(offset, offset + 32)),
      x: Uint8Array.from(data.subarray(offset + 32, offset + 64)),
      y: Uint8Array.from(data.subarray(offset + 64, offset + 96)),
    };
  };
  return {
    members: Array.from({ length: nodeCount }, (_, i) => record(i)),
    providers: Array.from({ length: providerCount }, (_, i) =>
      record(nodeCount + i),
    ),
  };
}

/** A registry snapshot: its decoded header fields, and its two member tables. */
export type RegistryAccount = RegistryLike & {
  version: number;
  redundancyBuffer: number;
  providerCount: number;
  providers: RegistryMemberLike[];
  [field: string]: unknown;
};

/** Fetch a registry snapshot with its members, or `null` if the account is not a registry. */
export async function fetchRegistry(
  program: Program,
  address: PublicKey,
): Promise<RegistryAccount | null> {
  const info = await program.provider.connection.getAccountInfo(address);
  if (!info) return null;
  let header: any;
  try {
    header = program.coder.accounts.decode("registry", info.data);
  } catch (_error) {
    return null;
  }
  return {
    ...header,
    nodeCount: Number(header.nodeCount),
    providerCount: Number(header.providerCount),
    ...decodeRegistryMembers(
      info.data,
      Number(header.nodeCount),
      Number(header.providerCount),
    ),
  };
}

/**
 * How many `extend_registry` instructions must precede one that publishes a registry version with
 * room for `members` members, given how much of that version's account is already allocated.
 *
 * The publishing instruction adds up to 10 KiB itself, so a snapshot of up to 106 members needs
 * none; up to 213 needs one, and up to 256 two.
 */
export function registryExtendsNeeded(
  members: number,
  allocatedLen = 0,
): number {
  const short = Math.max(0, registrySpace(members) - allocatedLen);
  return short === 0 ? 0 : Math.floor((short - 1) / MAX_GROWTH_PER_INSTRUCTION);
}

/**
 * The `extend_registry` instructions to put in front of an instruction that publishes the version
 * after `currentVersion` with room for `members` members. Empty for a registry of up to 106.
 *
 * `members` is the largest table, in records (node and provider slots together), the instruction
 * works in: the current count for a rotation, a policy change or a removal, one more for a node
 * registration or a reinstatement, and `providerRegistrationMembers` for a provider registration.
 */
export async function registryExtendInstructions(
  program: Program,
  payer: PublicKey,
  currentVersion: number,
  members: number,
): Promise<TransactionInstruction[]> {
  const programId = program.programId;
  const [newRegistry] = findRegistryPda(programId, currentVersion + 1);
  const allocated =
    (await program.provider.connection.getAccountInfo(newRegistry))?.data
      .length ?? 0;
  const records = Math.min(members, MAX_REGISTRY_NODES);
  const count = registryExtendsNeeded(records, allocated);
  if (count === 0) return [];
  const instruction: TransactionInstruction = await (program.methods as any)
    // The record count the publishing instruction needs room for (at least one more than the
    // current version has).
    .extendRegistry(records)
    .accountsPartial({
      payer,
      registryState: findRegistryStatePda(programId)[0],
      currentRegistry: findRegistryPda(programId, currentVersion)[0],
      newRegistry,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  return Array.from({ length: count }, () => instruction);
}

/**
 * Records the version published by registering a provider into `slot` needs room for: the
 * current records, plus the provider slots up to and including `slot` that the table does not
 * have yet (reserved slot 0 too, for the first provider). A freed slot adds none.
 */
export function providerRegistrationMembers(
  registry: RegistryLike,
  slot: number,
): number {
  const providers = Number(registry.providerCount ?? 0);
  return registryRecords(registry) + Math.max(0, slot + 1 - providers);
}
