import * as crypto from "crypto";
import { BN, Program } from "@coral-xyz/anchor";
import {
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { keccak_256 } from "@noble/hashes/sha3";
import { TestContext } from "../setup";

const DOMAIN_REQAUTH = Buffer.from("MOLPHA_REQAUTH_V1", "utf8");
const DOMAIN_GWRCPT = Buffer.from("MOLPHA_GWRCPT_V1", "utf8");
const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1", "utf8"))
);
const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1", "utf8");

export const ROUND_RECEIPT_SEED = Buffer.from("molpha_round");
export const JOB_SEED = Buffer.from("molpha_job");
export const FEED_SEED = Buffer.from("molpha_feed");
export const PLAN_SEED = Buffer.from("molpha_plan");
export const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
export const NODE_SEED = Buffer.from("molpha_node");

function u32le(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0, 0);
  return out;
}

function u64le(value: number | bigint): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(BigInt(value), 0);
  return out;
}

function u32be(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value >>> 0, 0);
  return out;
}

function i64AsU64be(value: number): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt.asUintN(64, BigInt(value)), 0);
  return out;
}

export function sha256(parts: Buffer[]): Buffer {
  const h = crypto.createHash("sha256");
  for (const p of parts) {
    h.update(p);
  }
  return h.digest();
}

export function deriveJobId(owner: PublicKey, apiConfigHash: Buffer): Buffer {
  return Buffer.from(
    keccak_256(Buffer.concat([JOB_ID_PREFIX, owner.toBuffer(), apiConfigHash]))
  );
}

/** EVM uint256 bitmap with the lowest `n` bits set. */
export function bitmapForFirstN(n: number): Buffer {
  const bm = Buffer.alloc(32, 0);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return bm;
}

export function bitmapBitSet(bitmap: Buffer, bit: number): boolean {
  const byteIndex = 31 - (bit >> 3);
  const bitMask = 1 << (bit & 7);
  return (bitmap[byteIndex] & bitMask) !== 0;
}

export function hashRequestAuth(jobId: Buffer, timestamp: number): Buffer {
  const body = Buffer.concat([Buffer.from(jobId), u64le(timestamp)]);
  return sha256([DOMAIN_REQAUTH, body]);
}

export function hashAggregatedSig(aggSig: {
  s: Buffer;
  rx: Buffer;
  ryParity: number;
  commitmentAddr: Buffer;
  signersBitmap: Buffer;
}): Buffer {
  const body = Buffer.concat([
    Buffer.from(aggSig.s),
    Buffer.from(aggSig.rx),
    Buffer.from([aggSig.ryParity]),
    Buffer.from(aggSig.commitmentAddr),
    Buffer.from(aggSig.signersBitmap),
  ]);
  return sha256([body]);
}

export function computeDataUpdateMessageHash(params: {
  jobId: Buffer;
  registryVersion: number;
  signaturesRequired: number;
  signersBitmap: Buffer;
  value: Buffer;
  timestamp: number;
}): Buffer {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        MESSAGE_PREFIX,
        Buffer.from(params.jobId),
        u32be(params.registryVersion),
        u32be(params.signaturesRequired),
        Buffer.from(params.signersBitmap),
        Buffer.from(params.value),
        i64AsU64be(params.timestamp),
      ])
    )
  );
}

export function hashGatewayReceipt(
  gatewayReceipt: {
    registryVersion: number;
    selectionBitmap: Buffer;
    gatewayIndex: number;
  },
  requestAuthHash: Buffer,
  aggSigHash: Buffer,
  dataUpdateMessageHash: Buffer
): Buffer {
  const body = Buffer.concat([
    u32le(gatewayReceipt.registryVersion),
    Buffer.from(gatewayReceipt.selectionBitmap),
    u32le(gatewayReceipt.gatewayIndex),
  ]);
  return sha256([
    DOMAIN_GWRCPT,
    body,
    requestAuthHash,
    aggSigHash,
    dataUpdateMessageHash,
  ]);
}

export function hashCreditedNodes(nodePubkeys: PublicKey[]): Buffer {
  return sha256(nodePubkeys.map((pk) => pk.toBuffer()));
}

export function findRoundRecordPda(
  programId: PublicKey,
  jobId: Buffer,
  timestamp: number
): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [ROUND_RECEIPT_SEED, jobId, u64le(timestamp)],
    programId
  );
  return pda;
}

export function settleRemainingAccounts(
  bitmap: Buffer,
  registryIndexPdasByIndex: Map<number, PublicKey>,
  nodePdasByIndex: Map<number, PublicKey>
): { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[] {
  const remaining: { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[] =
    [];
  for (let bit = 0; bit < 256; bit++) {
    if (!bitmapBitSet(bitmap, bit)) continue;
    const registryIndexPda = registryIndexPdasByIndex.get(bit);
    const nodePda = nodePdasByIndex.get(bit);
    if (!registryIndexPda || !nodePda) {
      throw new Error(`Missing node accounts for credited bit ${bit}`);
    }
    remaining.push({
      pubkey: registryIndexPda,
      isSigner: false,
      isWritable: false,
    });
    remaining.push({
      pubkey: nodePda,
      isSigner: false,
      isWritable: true,
    });
  }
  return remaining;
}

export function finalizeRemainingAccounts(
  bitmap: Buffer,
  nodePdasByIndex: Map<number, PublicKey>
): { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[] {
  const remaining: { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[] =
    [];
  for (let bit = 0; bit < 256; bit++) {
    if (!bitmapBitSet(bitmap, bit)) continue;
    const nodePda = nodePdasByIndex.get(bit);
    if (!nodePda) {
      throw new Error(`Missing node account for credited bit ${bit}`);
    }
    remaining.push({
      pubkey: nodePda,
      isSigner: false,
      isWritable: true,
    });
  }
  return remaining;
}

export async function sendSettleRoundReceipt(
  ctx: TestContext,
  program: Program,
  params: {
    payer: Keypair;
    protocolConfigPda: PublicKey;
    registryStatePda: PublicKey;
    jobPda: PublicKey;
    subscriptionPda: PublicKey;
    gatewayPda: PublicKey;
    gatewayAuthority: Keypair;
    jobId: Buffer;
    registryVersion: number;
    gatewayIndex: number;
    signaturesRequired: number;
    creditedBitmap: Buffer;
    timestamp: number;
    value: Buffer;
    remainingAccounts: { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[];
  }
): Promise<PublicKey> {
  const roundRecordPda = findRoundRecordPda(
    program.programId,
    params.jobId,
    params.timestamp
  );

  const aggSig = {
    s: crypto.randomBytes(32),
    rx: crypto.randomBytes(32),
    ryParity: 1,
    commitmentAddr: crypto.randomBytes(20),
    signersBitmap: params.creditedBitmap,
  };

  const gatewayReceipt = {
    registryVersion: params.registryVersion,
    selectionBitmap: params.creditedBitmap,
    gatewayIndex: params.gatewayIndex,
  };

  const requestAuthHash = hashRequestAuth(params.jobId, params.timestamp);
  const aggSigHash = hashAggregatedSig(aggSig);
  const dataUpdateMessageHash = computeDataUpdateMessageHash({
    jobId: params.jobId,
    registryVersion: gatewayReceipt.registryVersion,
    signaturesRequired: params.signaturesRequired,
    signersBitmap: aggSig.signersBitmap,
    value: params.value,
    timestamp: params.timestamp,
  });
  const gatewayReceiptHash = hashGatewayReceipt(
    gatewayReceipt,
    requestAuthHash,
    aggSigHash,
    dataUpdateMessageHash
  );

  const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
    privateKey: params.payer.secretKey,
    message: requestAuthHash,
  });
  const gatewaySigIx = Ed25519Program.createInstructionWithPrivateKey({
    privateKey: params.gatewayAuthority.secretKey,
    message: gatewayReceiptHash,
  });

  const settleIx: TransactionInstruction = await program.methods
    .settleRoundReceipt({
      jobId: Array.from(params.jobId),
      timestamp: new BN(params.timestamp),
      aggSig: {
        s: Array.from(aggSig.s),
        rx: Array.from(aggSig.rx),
        ryParity: aggSig.ryParity,
        commitmentAddr: Array.from(aggSig.commitmentAddr),
        signersBitmap: Array.from(aggSig.signersBitmap),
      },
      gatewayReceipt: {
        registryVersion: gatewayReceipt.registryVersion,
        selectionBitmap: Array.from(gatewayReceipt.selectionBitmap),
        gatewayIndex: gatewayReceipt.gatewayIndex,
      },
      value: Array.from(params.value),
      consumerAuthority: params.payer.publicKey,
    })
    .accountsPartial({
      payer: params.payer.publicKey,
      protocolConfig: params.protocolConfigPda,
      registryState: params.registryStatePda,
      job: params.jobPda,
      subscription: params.subscriptionPda,
      consumerDelegate: null,
      gateway: params.gatewayPda,
      roundRecord: roundRecordPda,
      instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(params.remainingAccounts)
    .instruction();

  const tx = new Transaction().add(consumerSigIx, gatewaySigIx, settleIx);
  await ctx.provider.sendAndConfirm(tx, [params.payer]);

  return roundRecordPda;
}
