import { PublicKey } from "@solana/web3.js";
import { u32Le } from "./utils";

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
export const NODE_SEED = Buffer.from("molpha_node");
export const PLAN_SEED = Buffer.from("molpha_plan");
export const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
export const JOB_SEED = Buffer.from("molpha_job");
export const FEED_SEED = Buffer.from("molpha_feed");
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

export function findRegistryIndexPda(
  programId: PublicKey,
  index: number
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [REGISTRY_INDEX_SEED, u32Le(index)],
    programId
  );
}

export function findVirtualRegistryIndexPda(
  programId: PublicKey
): [PublicKey, number] {
  return findRegistryIndexPda(programId, VIRTUAL_INDEX);
}

export function findNodePda(
  programId: PublicKey,
  nodeId: Uint8Array
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, Buffer.from(nodeId)],
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

export function findJobPda(
  programId: PublicKey,
  jobId: Uint8Array
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [JOB_SEED, Buffer.from(jobId)],
    programId
  );
}

export function findFeedPda(
  programId: PublicKey,
  jobId: Uint8Array
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [FEED_SEED, Buffer.from(jobId)],
    programId
  );
}
