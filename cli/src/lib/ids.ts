import { PublicKey } from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3";

export const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
export const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");

export function deriveNodeId(
  owner: PublicKey,
  secp256k1PubkeyCompressed: Uint8Array,
  registrationSalt: Uint8Array
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      NODE_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(secp256k1PubkeyCompressed),
      Buffer.from(registrationSalt),
    ])
  );
}

export function deriveJobId(
  owner: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      JOB_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired]),
    ])
  );
}
