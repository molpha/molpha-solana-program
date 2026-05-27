import { readJsonFile } from "../utils";

export type VerifyDataUpdateFixture = {
  registeredNodeCount: number;
  registryVersion: number;
  configHash: string;
  nodeIndexesOneBased: number[];
  nodePubkeys: string[];
  secretKeys?: string[];
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

export function loadVerifyFixture(filePath: string): VerifyDataUpdateFixture {
  return readJsonFile<VerifyDataUpdateFixture>(filePath);
}
