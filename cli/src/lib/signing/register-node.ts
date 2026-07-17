import { keccak_256 } from "@noble/hashes/sha3";

export function buildRegisterNodeArgs(params: {
  registrationSalt: Uint8Array;
  secp256k1PubkeyCompressed: Uint8Array;
  popSigR: Uint8Array;
  popSigS: Uint8Array;
  ip?: number[];
  port?: number;
}) {
  if (params.secp256k1PubkeyCompressed.length !== 33) {
    throw new Error("secp256k1 public key must be 33 bytes");
  }
  if (params.popSigR.length !== 32 || params.popSigS.length !== 32) {
    throw new Error("PoP r and s must each be 32 bytes");
  }
  return {
    registrationSalt: Array.from(params.registrationSalt),
    ip: params.ip ?? [127, 0, 0, 1],
    port: params.port ?? 8080,
    secp256K1PubkeyCompressed: Array.from(params.secp256k1PubkeyCompressed),
    secp256K1KeyFingerprint: Array.from(
      keccak_256(params.secp256k1PubkeyCompressed)
    ),
    popSigR: Array.from(params.popSigR),
    popSigS: Array.from(params.popSigS),
  };
}
