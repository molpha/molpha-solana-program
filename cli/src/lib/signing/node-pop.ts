import { PublicKey } from "@solana/web3.js";
import { Point, utils as secpUtils } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3.js";

export const NODE_POP_PREFIX = Buffer.from("MOLPHA_NODE_POP_V1");
export const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
);

function modN(value: bigint): bigint {
  const reduced = value % SECP256K1_ORDER_N;
  return reduced >= 0n ? reduced : reduced + SECP256K1_ORDER_N;
}

function bigintFromBytesBE(bytes: Uint8Array): bigint {
  return BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
}

function bytes32FromBigintBE(value: bigint): Uint8Array {
  const hex = modN(value).toString(16).padStart(64, "0");
  return Uint8Array.from(Buffer.from(hex, "hex"));
}

export function deriveNodePopMessage(
  programId: PublicKey,
  nodeId: Uint8Array,
): Uint8Array {
  if (nodeId.length !== 32) throw new Error("node id must be 32 bytes");
  return keccak_256(
    Buffer.concat([NODE_POP_PREFIX, programId.toBuffer(), Buffer.from(nodeId)]),
  );
}

export type NodePopSignature = {
  secp256k1PubkeyCompressed: Uint8Array;
  popSigR: Uint8Array;
  popSigS: Uint8Array;
};

/** Sign the domain-separated Molpha node PoP with a CSPRNG nonce by default. */
export function signNodePop(params: {
  programId: PublicKey;
  nodeId: Uint8Array;
  secp256k1PrivateKey: Uint8Array;
  /** Test-vector hook. Production callers must leave this unset. */
  nonce?: Uint8Array;
}): NodePopSignature {
  if (params.secp256k1PrivateKey.length !== 32) {
    throw new Error("secp256k1 private key must be 32 bytes");
  }
  const secret = bigintFromBytesBE(params.secp256k1PrivateKey);
  if (secret <= 0n || secret >= SECP256K1_ORDER_N) {
    throw new Error("invalid secp256k1 private key scalar");
  }

  const publicPoint = Point.BASE.multiply(secret);
  const compressed = publicPoint.toRawBytes(true);
  const message = deriveNodePopMessage(params.programId, params.nodeId);

  let nonce = bigintFromBytesBE(params.nonce ?? secpUtils.randomPrivateKey());
  nonce = modN(nonce);
  if (nonce === 0n) throw new Error("PoP nonce must not be zero");
  let noncePoint = Point.BASE.multiply(nonce);
  if ((noncePoint.toRawBytes(true)[0] & 1) === 1) {
    nonce = SECP256K1_ORDER_N - nonce;
    noncePoint = Point.BASE.multiply(nonce);
  }
  const nonceCompressed = noncePoint.toRawBytes(true);
  if (nonceCompressed[0] !== 0x02) {
    throw new Error("failed to canonicalize PoP nonce to even Y");
  }
  const r = nonceCompressed.subarray(1, 33);
  const parity = Uint8Array.from([compressed[0] & 1]);
  const challenge = modN(
    bigintFromBytesBE(
      keccak_256(
        Buffer.concat([
          Buffer.from(compressed.subarray(1, 33)),
          Buffer.from(parity),
          Buffer.from(message),
          Buffer.from(r),
        ]),
      ),
    ),
  );
  const s = modN(nonce + challenge * secret);
  if (s === 0n) throw new Error("generated zero PoP response scalar");

  return {
    secp256k1PubkeyCompressed: Uint8Array.from(compressed),
    popSigR: Uint8Array.from(r),
    popSigS: bytes32FromBigintBE(s),
  };
}
