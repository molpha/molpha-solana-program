"use strict";

const { PublicKey } = require("@solana/web3.js");
const { Point, utils: secpUtils } = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");

const NODE_POP_PREFIX = Buffer.from("MOLPHA_NODE_POP_V1");
const SECP256K1_KEY_CLAIM_SEED = Buffer.from("molpha_secp_key");
const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

function modN(value) {
  const reduced = value % SECP256K1_ORDER_N;
  return reduced >= 0n ? reduced : reduced + SECP256K1_ORDER_N;
}

function bigintFromBytesBE(bytes) {
  return BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
}

function bytes32FromBigintBE(value) {
  return Buffer.from(modN(value).toString(16).padStart(64, "0"), "hex");
}

function deriveNodePopMessage(programId, nodeId) {
  if (!(programId instanceof PublicKey)) throw new Error("invalid program id");
  if (nodeId.length !== 32) throw new Error("node id must be 32 bytes");
  return Buffer.from(
    keccak_256(Buffer.concat([NODE_POP_PREFIX, programId.toBuffer(), nodeId]))
  );
}

function signNodePop({ programId, nodeId, secp256k1PrivateKey, nonce }) {
  if (secp256k1PrivateKey.length !== 32) {
    throw new Error("secp256k1 private key must be 32 bytes");
  }
  const secret = bigintFromBytesBE(secp256k1PrivateKey);
  if (secret <= 0n || secret >= SECP256K1_ORDER_N) {
    throw new Error("invalid secp256k1 private key scalar");
  }
  const publicPoint = Point.BASE.multiply(secret);
  const compressed = Buffer.from(publicPoint.toRawBytes(true));
  const message = deriveNodePopMessage(programId, nodeId);

  let nonceScalar = modN(
    bigintFromBytesBE(nonce ?? secpUtils.randomPrivateKey())
  );
  if (nonceScalar === 0n) throw new Error("PoP nonce must not be zero");
  let noncePoint = Point.BASE.multiply(nonceScalar);
  if ((noncePoint.toRawBytes(true)[0] & 1) === 1) {
    nonceScalar = SECP256K1_ORDER_N - nonceScalar;
    noncePoint = Point.BASE.multiply(nonceScalar);
  }
  const nonceCompressed = Buffer.from(noncePoint.toRawBytes(true));
  const r = nonceCompressed.subarray(1, 33);
  const challenge = modN(
    bigintFromBytesBE(
      keccak_256(
        Buffer.concat([
          compressed.subarray(1, 33),
          Buffer.from([compressed[0] & 1]),
          message,
          r,
        ])
      )
    )
  );
  const s = modN(nonceScalar + challenge * secret);
  if (s === 0n) throw new Error("generated zero PoP response scalar");
  return {
    secp256k1PubkeyCompressed: compressed,
    popSigR: Buffer.from(r),
    popSigS: bytes32FromBigintBE(s),
  };
}

function findSecp256k1KeyClaimPda(programId, compressedPubkey) {
  const fingerprint = Buffer.from(keccak_256(compressedPubkey));
  return PublicKey.findProgramAddressSync(
    [SECP256K1_KEY_CLAIM_SEED, fingerprint],
    programId
  )[0];
}

module.exports = {
  NODE_POP_PREFIX,
  SECP256K1_KEY_CLAIM_SEED,
  deriveNodePopMessage,
  findSecp256k1KeyClaimPda,
  signNodePop,
};
