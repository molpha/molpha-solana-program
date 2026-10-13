import { PublicKey } from "@solana/web3.js";
import { Point } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3.js";
import { schnorrSignForVerifier } from "./attestation";
import { SECP256K1_ORDER_N, signNodePop } from "./node-pop";

export const NODE_KEY_ROTATION_PREFIX = Buffer.from(
  "MOLPHA_NODE_KEY_ROTATION_V1",
);

/** Anchor `KeyRotationProof`, the one argument of `rotate_node_key`. */
export type KeyRotationProofArgs = {
  newPublicKeyCompressed: number[];
  oldKeySigS: number[];
  oldKeyCommitment: number[];
  popSigR: number[];
  popSigS: number[];
};

/**
 * The digest both keys sign to rotate `node`'s signing key to `newKeyCompressed`:
 * `keccak256("MOLPHA_NODE_KEY_ROTATION_V1" ‖ program_id ‖ node ‖ new_key_compressed)`.
 *
 * `node` is the node account address (`findNodePda`), not the owner.
 */
export function deriveKeyRotationDigest(
  programId: PublicKey,
  node: PublicKey,
  newKeyCompressed: Uint8Array,
): Uint8Array {
  if (newKeyCompressed.length !== 33) {
    throw new Error("secp256k1 public key must be 33 bytes");
  }
  return keccak_256(
    Buffer.concat([
      NODE_KEY_ROTATION_PREFIX,
      programId.toBuffer(),
      node.toBuffer(),
      Buffer.from(newKeyCompressed),
    ]),
  );
}

function scalarFromPrivateKey(key: Uint8Array, label: string): bigint {
  if (key.length !== 32) throw new Error(`${label} must be 32 bytes`);
  const scalar = BigInt(`0x${Buffer.from(key).toString("hex")}`);
  if (scalar <= 0n || scalar >= SECP256K1_ORDER_N) {
    throw new Error(`invalid ${label} scalar`);
  }
  return scalar;
}

/**
 * Both signatures a rotation needs: the current key signs the digest in the aggregate format,
 * and the new key proves possession over it (the digest takes the place of the node id in a
 * registration proof).
 *
 * This holds both private keys, so it belongs in the node operator's own tooling. The `registry
 * rotate-key` command takes the resulting signatures, never a key.
 */
export function signKeyRotation(params: {
  programId: PublicKey;
  node: PublicKey;
  currentPrivateKey: Uint8Array;
  newPrivateKey: Uint8Array;
  /** Test-vector hooks. Production callers must leave these unset. */
  nonces?: { current?: Uint8Array; pop?: Uint8Array };
}): KeyRotationProofArgs {
  const currentSecret = scalarFromPrivateKey(
    params.currentPrivateKey,
    "current secp256k1 private key",
  );
  scalarFromPrivateKey(params.newPrivateKey, "new secp256k1 private key");

  const currentPublic = Point.BASE.multiply(currentSecret).toRawBytes(true);
  const newPublic = Point.BASE.multiply(
    scalarFromPrivateKey(params.newPrivateKey, "new secp256k1 private key"),
  ).toRawBytes(true);
  const digest = deriveKeyRotationDigest(
    params.programId,
    params.node,
    newPublic,
  );

  const current = schnorrSignForVerifier({
    coalitionPubkeyCompressed: currentPublic,
    messageHash: digest,
    coalitionPrivkey: currentSecret,
    nonce: params.nonces?.current,
  });
  const pop = signNodePop({
    programId: params.programId,
    nodeId: digest,
    secp256k1PrivateKey: params.newPrivateKey,
    nonce: params.nonces?.pop,
  });

  return buildKeyRotationArgs({
    newPublicKeyCompressed: newPublic,
    oldKeySigS: current.s,
    oldKeyCommitment: current.commitment,
    popSigR: pop.popSigR,
    popSigS: pop.popSigS,
  });
}

/** Validate and shape the `rotate_node_key` argument from raw signature bytes. */
export function buildKeyRotationArgs(params: {
  newPublicKeyCompressed: Uint8Array;
  oldKeySigS: Uint8Array;
  oldKeyCommitment: Uint8Array;
  popSigR: Uint8Array;
  popSigS: Uint8Array;
}): KeyRotationProofArgs {
  if (params.newPublicKeyCompressed.length !== 33) {
    throw new Error("new secp256k1 public key must be 33 bytes");
  }
  if (params.oldKeySigS.length !== 32) {
    throw new Error("old key signature s must be 32 bytes");
  }
  if (params.oldKeyCommitment.length !== 20) {
    throw new Error("old key signature commitment must be 20 bytes");
  }
  if (params.popSigR.length !== 32 || params.popSigS.length !== 32) {
    throw new Error("PoP r and s must each be 32 bytes");
  }
  return {
    newPublicKeyCompressed: Array.from(params.newPublicKeyCompressed),
    oldKeySigS: Array.from(params.oldKeySigS),
    oldKeyCommitment: Array.from(params.oldKeyCommitment),
    popSigR: Array.from(params.popSigR),
    popSigS: Array.from(params.popSigS),
  };
}
