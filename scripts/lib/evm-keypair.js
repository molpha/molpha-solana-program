"use strict";

const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3.js");

/**
 * Generate a secp256k1 node signing key and its EVM address
 * (last 20 bytes of keccak256 over the uncompressed key without its 0x04 prefix).
 */
function generateKeypair() {
  const privateKey = secp256k1.utils.randomPrivateKey();
  const point = secp256k1.ProjectivePoint.fromPrivateKey(privateKey);
  const uncompressed = point.toRawBytes(false);
  const compressed = point.toRawBytes(true);
  const address = keccak_256(uncompressed.slice(1)).slice(-20);

  return {
    privateKey: Buffer.from(privateKey),
    publicKeyUncompressed: Buffer.from(uncompressed),
    publicKeyCompressed: Buffer.from(compressed),
    address: Buffer.from(address),
  };
}

module.exports = { generateKeypair };
