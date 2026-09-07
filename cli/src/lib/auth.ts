import { keccak_256 } from "@noble/hashes/sha3.js";
import { PublicKey } from "@solana/web3.js";
import { u64Le } from "./utils";

const DOMAIN_REQAUTH = Buffer.from("MOLPHA_REQAUTH_V1", "utf8");

/**
 * `H(RequestAuth)` — domain-separated Keccak256 over borsh
 * `(program_id, gateway, source_id, signatures_required, timestamp)`.
 *
 * `programId` and `gateway` scope the authorization: the signature is only redeemable by the
 * gateway the consumer chose, on the deployment they meant to pay.
 */
export function hashRequestAuthFields(
  programId: PublicKey,
  gateway: PublicKey,
  sourceId: Uint8Array,
  signaturesRequired: number,
  timestamp: number | bigint,
): Buffer {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        DOMAIN_REQAUTH,
        programId.toBuffer(),
        gateway.toBuffer(),
        Buffer.from(sourceId),
        Buffer.from([signaturesRequired & 0xff]),
        u64Le(timestamp),
      ]),
    ),
  );
}
