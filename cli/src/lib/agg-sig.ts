import { hexToBytes, parseInteger, readJsonFile } from "./utils";
import {
  evmBitmapBitSet,
  evmUint256BitmapForFirstN,
  SchnorrSignatureBytes,
} from "./signing/attestation";

function bytesFromField(
  value: string | number[] | Uint8Array,
  expectedLength: number,
  label: string,
): Uint8Array {
  if (typeof value === "string") {
    return hexToBytes(value, expectedLength, label);
  }
  const bytes = Uint8Array.from(value);
  if (bytes.length !== expectedLength) {
    throw new Error(
      `${label} must be ${expectedLength} bytes, got ${bytes.length}`,
    );
  }
  return bytes;
}

export function bitmapFromBits(bits: number[]): Uint8Array {
  const bitmap = new Uint8Array(32);
  for (const bit of bits) {
    bitmap[31 - (bit >> 3)] |= 1 << (bit & 7);
  }
  return bitmap;
}

export function creditedBitsFromBitmap(signersBitmap: Uint8Array): number[] {
  const bits: number[] = [];
  for (let bit = 0; bit < 256; bit += 1) {
    if (evmBitmapBitSet(signersBitmap, bit)) {
      bits.push(bit);
    }
  }
  return bits;
}

/** Parse a JSON `SchnorrSignature` file; omitted `aggSigS` / `commitment` stay unset. */
export function parseSchnorrSignatureJson(
  filePath: string,
): SchnorrSignatureBytes {
  const raw = readJsonFile<{
    aggSigS?: string | number[];
    /** @deprecated Prefer `aggSigS`. */
    s?: string | number[];
    commitment?: string | number[];
    signersBitmap: string | number[];
  }>(filePath);
  const aggSigS = raw.aggSigS ?? raw.s;

  return {
    signersBitmap: bytesFromField(raw.signersBitmap, 32, "signers bitmap"),
    aggSigS: aggSigS ? bytesFromField(aggSigS, 32, "agg sig s") : undefined,
    commitment: raw.commitment
      ? bytesFromField(raw.commitment, 20, "commitment")
      : undefined,
  };
}

export function signersBitmapFromOptions(options: {
  signersBitmap?: string;
  creditedBits?: string;
  signerCount?: string;
}): Uint8Array {
  if (options.signersBitmap) {
    return hexToBytes(options.signersBitmap, 32, "signers bitmap");
  }
  if (options.creditedBits) {
    const bits = options.creditedBits
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => parseInteger(part, "credited bit"));
    return bitmapFromBits(bits);
  }
  if (options.signerCount) {
    return evmUint256BitmapForFirstN(
      parseInteger(options.signerCount, "signer count"),
    );
  }
  throw new Error(
    "Provide --signers-bitmap, --credited-bits, or --signer-count",
  );
}

export function firstNSignersBitmap(n: number): Uint8Array {
  return evmUint256BitmapForFirstN(n);
}
