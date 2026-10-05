import { hexToBytes, readJsonFile } from "./utils";
import { SchnorrSignatureBytes } from "./signing/attestation";

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

/** Parse a JSON `SchnorrSignature` file; omitted `aggSigS` / `commitment` stay unset. */
export function parseSchnorrSignatureJson(
  filePath: string,
): SchnorrSignatureBytes {
  const raw = readJsonFile<{
    aggSigS?: string | number[];
    commitment?: string | number[];
    signersBitmap: string | number[];
  }>(filePath);

  return {
    signersBitmap: bytesFromField(raw.signersBitmap, 32, "signers bitmap"),
    aggSigS: raw.aggSigS
      ? bytesFromField(raw.aggSigS, 32, "agg sig s")
      : undefined,
    commitment: raw.commitment
      ? bytesFromField(raw.commitment, 20, "commitment")
      : undefined,
  };
}
