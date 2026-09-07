import { readJsonFile } from "../utils";

export type VerifyAttestationFixture = {
  registeredNodeCount: number;
  registryVersion: number;
  configHash: string;
  nodeIndexesOneBased: number[];
  nodePubkeys: string[];
  secretKeys?: string[];
  nodePrivkeys?: string[];
  dataUpdate: {
    sourceId?: string;
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

export function fixtureSourceId(fixture: VerifyAttestationFixture): string {
  const sourceId = fixture.dataUpdate.sourceId;
  if (!sourceId) {
    throw new Error("fixture dataUpdate.sourceId is missing");
  }
  return sourceId;
}

export function loadVerifyFixture(filePath: string): VerifyAttestationFixture {
  return readJsonFile<VerifyAttestationFixture>(filePath);
}
