import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import { keccak_256 } from "@noble/hashes/sha3";
import {
  bytesToHex0x,
  deriveJobId,
  E2eDataUpdateFields,
  evmSignersBitmapNumber,
  E2eSignerMaterial,
  SignedE2eDataUpdate,
} from "./evm-submit-data-update";

/** JSON shape compatible with `tests/fixtures-json/verify-answer-evm.json`. */
export type VerifyAnswerEvmFixture = {
  registeredNodeCount: number;
  registryVersion: number;
  configHash: string;
  nodeIndexesOneBased: number[];
  nodePubkeys: string[];
  /** Decimal-string secp256k1 scalars for the signing coalition (EVM export style). */
  secretKeys?: string[];
  /** Hex-encoded privkeys from bankrun e2e export (optional). */
  nodePrivkeys?: string[];
  dataUpdate: {
    jobId: string;
    registryVersion: number;
    signaturesRequired: number;
    configHash: string;
    value: string;
    canonicalTimestamp: number;
  };
  schnorrSignature: {
    signature: string;
    commitment: string;
    signersBitmap: number;
  };
};

export type E2eScenario = E2eDataUpdateFields & {
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
    configHash: labeledBytes32(seed, "apiConfigHash"),
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
  configHash: Uint8Array;
  nodePubkeys: Uint8Array[];
  nodePrivkeys: bigint[];
  signerCount: number;
  jobId: Uint8Array;
  dataUpdate: E2eDataUpdateFields;
  signed: Pick<SignedE2eDataUpdate, "aggSigS" | "commitmentAddr">;
}): VerifyAnswerEvmFixture {
  const nodeIndexesOneBased = Array.from(
    { length: params.signerCount },
    (_, i) => i + 1
  );

  return {
    registeredNodeCount: params.registeredNodeCount,
    registryVersion: params.registryVersion,
    configHash: bytesToHex0x(params.configHash),
    nodeIndexesOneBased,
    nodePubkeys: params.nodePubkeys.map((pk) => bytesToHex0x(pk)),
    secretKeys: params.nodePrivkeys.map((sk) => sk.toString()),
    dataUpdate: {
      jobId: bytesToHex0x(params.jobId),
      registryVersion: params.registryVersion,
      signaturesRequired: params.signerCount,
      configHash: bytesToHex0x(params.dataUpdate.configHash),
      value: bytesToHex0x(params.dataUpdate.value),
      canonicalTimestamp: params.dataUpdate.canonicalTimestamp,
    },
    schnorrSignature: {
      signature: bytesToHex0x(params.signed.aggSigS),
      commitment: bytesToHex0x(params.signed.commitmentAddr),
      signersBitmap: evmSignersBitmapNumber(params.signerCount),
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

export function jobIdForOwner(owner: import("@solana/web3.js").PublicKey, configHash: Uint8Array) {
  return deriveJobId(owner, configHash);
}
