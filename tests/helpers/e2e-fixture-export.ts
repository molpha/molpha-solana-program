import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import { keccak_256 } from "@noble/hashes/sha3";
import {
  bytesToHex0x,
  E2eAttestationFields,
  evmSignersBitmapNumberFromBytes,
  E2eSignerMaterial,
  SignedE2eAttestation,
} from "./submit-attestation";

/** JSON shape compatible with `tests/fixtures-json/verify-answer-evm.json`. */
export type VerifyAnswerEvmFixture = {
  registeredNodeCount: number;
  registryVersion: number;
  messageHash: string;
  nodeIndexesOneBased: number[];
  nodePubkeys: string[];
  /** Decimal-string secp256k1 scalars for the signing coalition (EVM export style). */
  secretKeys?: string[];
  /** Hex-encoded privkeys from bankrun e2e export (optional). */
  nodePrivkeys?: string[];
  attestation: {
    sourceId: string;
    registryVersion: number;
    signaturesRequired: number;
    value: string;
    canonicalTimestamp: number;
  };
  schnorrSignature: {
    signature: string;
    commitment: string;
    signersBitmap: number;
  };
};

export type E2eScenario = E2eAttestationFields & {
  sourceId: Uint8Array;
  configHash: Uint8Array;
  seed: string;
  signerCount: number;
};

function labeledBytes32(seed: string, label: string): Uint8Array {
  return keccak_256(Buffer.concat([Buffer.from(seed), Buffer.from(label)]));
}

/** Deterministic random-ish payload from a seed (not the static EVM fixture). */
export function generateE2eScenario(
  seed: string,
  signerCount = 8
): E2eScenario {
  const tsDigest = labeledBytes32(seed, "canonicalTimestamp");
  const tsLo = Buffer.from(tsDigest.subarray(28, 32)).readUInt32BE(0);
  const ts = 1_700_000_000 + (tsLo % 10_000_000);

  return {
    seed,
    signerCount,
    sourceId: labeledBytes32(seed, "sourceId"),
    configHash: labeledBytes32(seed, "configHash"),
    value: labeledBytes32(seed, "feedValue"),
    canonicalTimestamp: ts,
  };
}

export function randomE2eScenarioSeed(): string {
  return crypto.randomBytes(16).toString("hex");
}

export function buildVerifyAnswerEvmFixture(params: {
  registeredNodeCount: number;
  registryVersion: number;
  messageHash: Uint8Array;
  nodePubkeys: Uint8Array[];
  nodePrivkeys: bigint[];
  signaturesRequired: number;
  sourceId: Uint8Array;
  attestation: E2eAttestationFields;
  signed: Pick<
    SignedE2eAttestation,
    "aggSigS" | "commitmentAddr" | "signersBitmap"
  >;
}): VerifyAnswerEvmFixture {
  const signersBitmap = evmSignersBitmapNumberFromBytes(
    params.signed.signersBitmap
  );
  const nodeIndexesOneBased = bitmapToNodeIndexesOneBased(BigInt(signersBitmap));

  return {
    registeredNodeCount: params.registeredNodeCount,
    registryVersion: params.registryVersion,
    messageHash: bytesToHex0x(params.messageHash),
    nodeIndexesOneBased,
    nodePubkeys: params.nodePubkeys.map((pk) => bytesToHex0x(pk)),
    secretKeys: params.nodePrivkeys.map((sk) => sk.toString()),
    attestation: {
      sourceId: bytesToHex0x(params.sourceId),
      registryVersion: params.registryVersion,
      signaturesRequired: params.signaturesRequired,
      value: bytesToHex0x(params.attestation.value),
      canonicalTimestamp: params.attestation.canonicalTimestamp,
    },
    schnorrSignature: {
      signature: bytesToHex0x(params.signed.aggSigS),
      commitment: bytesToHex0x(params.signed.commitmentAddr),
      signersBitmap,
    },
  };
}

export function writeVerifyAnswerEvmFixture(
  filePath: string,
  fixture: VerifyAnswerEvmFixture
): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(fixture, null, 2)}\n`, "utf8");
}

/** Resolve output path: `E2E_FIXTURE_OUTPUT` or default under fixtures-json. */
export function defaultE2eFixtureOutputPath(seed: string): string {
  const override = process.env.E2E_FIXTURE_OUTPUT;
  if (override) {
    return path.isAbsolute(override)
      ? override
      : path.join(process.cwd(), override);
  }
  const safeSeed = seed.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 16);
  return path.join(
    process.cwd(),
    "tests",
    "fixtures-json",
    `e2e-generated-${safeSeed}.json`
  );
}

function bitmapToNodeIndexesOneBased(bitmap: bigint): number[] {
  const indexes: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (((bitmap >> BigInt(bit)) & BigInt(1)) === BigInt(1)) {
      indexes.push(bit + 1);
    }
  }
  return indexes;
}