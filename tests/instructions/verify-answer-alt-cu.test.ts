import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import * as secp256k1 from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";

import {
  initializeProtocol,
  registerNodeWithDeposit,
  setupTestContext,
  TestContext,
  createTestDataSourceInfo,
  getDataSourcePda,
  getFeedPda,
} from "../setup";

// Must match `programs/molpha/src/instructions/verify_answer.rs`.
const EVM_ROUND_ID_PREFIX = Buffer.from([
  0x05, 0xcc, 0x9a, 0xbf, 0xa8, 0x3c, 0xdc, 0x2d, 0xff, 0xa2, 0x8a, 0xe0, 0xd4, 0x68,
  0x59, 0x5e, 0xce, 0xd0, 0x56, 0x4a, 0x16, 0xf6, 0xed, 0x9a, 0x3e, 0x2c, 0x5b, 0x4e,
  0x65, 0x2d, 0x98, 0x26,
]);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

function u32be(x: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(x >>> 0, 0);
  return b;
}

function u64be(x: bigint): Buffer {
  const b = Buffer.alloc(8);
  b.writeBigUInt64BE(x, 0);
  return b;
}

function u32le(x: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(x >>> 0, 0);
  return b;
}

function u64le(x: bigint): Buffer {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(x, 0);
  return b;
}

function bytes32FromAscii(s: string): Buffer {
  const out = Buffer.alloc(32);
  Buffer.from(s, "utf8").copy(out, 0, 0, Math.min(32, s.length));
  return out;
}

function evmUint256BitmapForFirstN(n: number): number[] {
  // EVM uint256 layout: bit0 is LSB of byte31.
  const bm = Buffer.alloc(32, 0);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return Array.from(bm);
}

function bigintFromBytesBE(b: Uint8Array): bigint {
  return BigInt("0x" + Buffer.from(b).toString("hex"));
}

function bytes32FromBigintBE(x: bigint): Buffer {
  let v = x;
  if (v < 0n) v = (v % SECP256K1_ORDER_N + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
  v %= SECP256K1_ORDER_N;
  const hex = v.toString(16).padStart(64, "0");
  return Buffer.from(hex, "hex");
}

function modN(x: bigint): bigint {
  const r = x % SECP256K1_ORDER_N;
  return r >= 0n ? r : r + SECP256K1_ORDER_N;
}

function keccakConcat(chunks: Buffer[]): Buffer {
  const preimage = Buffer.concat(chunks);
  return Buffer.from(keccak_256(preimage));
}

function commitmentAddrFromPoint(R: secp256k1.Point): Buffer {
  const uncompressed = Buffer.from(R.toRawBytes(false)); // 65: 0x04 || x || y
  const digest = Buffer.from(keccak_256(uncompressed.subarray(1))); // keccak(x||y)
  return digest.subarray(12, 32); // last 20 bytes
}

function schnorrSignForVerifier(opts: {
  coalitionPubkeyCompressed: Buffer; // 33
  messageHash: Buffer; // 32
  coalitionPrivkey: bigint; // mod n
}): { s: Buffer; commitmentAddr: Buffer } {
  const px = opts.coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = opts.coalitionPubkeyCompressed[0] & 1; // 0/1

  // Random nonce k in [1,n-1]
  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const R = secp256k1.Point.BASE.multiply(k);
  const commitmentAddr = commitmentAddrFromPoint(R);

  // challenge = keccak256(px || recoveryId || msg_hash || commitment_addr) mod n
  const challengeHash = keccakConcat([
    Buffer.from(px),
    Buffer.from([recoveryId]),
    opts.messageHash,
    commitmentAddr,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));

  // s = k + e*x mod n
  const s = modN(k + e * modN(opts.coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitmentAddr };
}

function serializeLookupTableAccountData(opts: {
  authority: PublicKey;
  // Set to a past slot so all addresses are active immediately.
  lastExtendedSlot: bigint;
  addresses: PublicKey[];
}): Buffer {
  const U64_MAX = 0xffff_ffff_ffff_ffffn;
  const meta = Buffer.concat([
    u32le(1), // typeIndex
    u64le(U64_MAX), // deactivationSlot
    u64le(opts.lastExtendedSlot), // lastExtendedSlot
    Buffer.from([0x00]), // lastExtendedStartIndex
    Buffer.from([0x01]), // option(Some authority)
    opts.authority.toBuffer(), // authority
    Buffer.from([0x00, 0x00]), // padding to 56
  ]);
  assert.strictEqual(meta.length, 56);
  const addrBytes = Buffer.concat(opts.addresses.map((a) => a.toBuffer()));
  return Buffer.concat([meta, addrBytes]);
}

describe("verify_answer_alt CU budgets (ALT/v0)", () => {
  it("measures CU usage for 3, 7, 18, 32 signers", async () => {
    const ctx: TestContext = await setupTestContext();
    await initializeProtocol(ctx);

    // Register 32 nodes once; reuse them across sub-cases.
    const nodePdas: PublicKey[] = [];
    const privKeys: bigint[] = [];
    for (let i = 0; i < 32; i++) {
      const skBytes = secp256k1.utils.randomPrivateKey();
      const sk = modN(bigintFromBytesBE(skBytes));
      privKeys.push(sk);
      const pkCompressed = Buffer.from(secp256k1.getPublicKey(skBytes, true)); // 33
      const nodePda = await registerNodeWithDeposit(ctx, ctx.nodes[i].publicKey, pkCompressed);
      nodePdas.push(nodePda);
    }

    const nodeCount = 32;
    const registryVersion = 1;
    const configHash = bytes32FromAscii("cu-config-alt");
    const valueBytes32 = bytes32FromAscii("cu-value-alt");
    const timestamp = 1_700_001_234n;

    async function measureForSignerCount(signerCount: number): Promise<number> {
      // Create a dedicated data source + feed with threshold == signerCount.
      const dataSourceName = `cu-alt-ds-${signerCount}`;
      const dataSourceInfo = createTestDataSourceInfo(0, dataSourceName);
      const [dataSourcePda] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        dataSourceName,
        0
      );
      try {
        await ctx.molphaProgram.methods
          .createDataSource(dataSourceInfo as any)
          .accountsPartial({
            authority: ctx.authority.publicKey,
            dataSource: dataSourcePda,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
      } catch (_e) {
        // ok in shared bankrun context
      }

      const jobId = bytes32FromAscii(`cu-alt-job-${signerCount}`);
      const [feedPda] = getFeedPda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        `cu-alt-feed-${signerCount}`,
        0
      );
      await ctx.molphaProgram.methods
        .createFeed(
          {
            name: `cu-alt-feed-${signerCount}`,
            jobId: Array.from(jobId),
            feedType: { public: {} },
            minSignaturesThreshold: signerCount,
            frequency: new BN(300),
            ipfsCid: "QmCuBudgetAlt",
          } as any,
          new BN(86400)
        )
        .accountsPartial({
          feed: feedPda,
          dataSource: dataSourcePda,
          authority: ctx.authority.publicKey,
          protocolConfig: ctx.protocolConfigPDA,
          userTokenAccount: ctx.userTokenAccount,
          programTokenAccount: ctx.programTokenAccount,
          underlyingToken: ctx.underlyingTokenMint,
          systemProgram: SystemProgram.programId,
          tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
          associatedTokenProgram: anchor.utils.token.ASSOCIATED_PROGRAM_ID,
        })
        .rpc();

      // Ensure deterministic group bitmap: group_size == node_count (full set).
      await ctx.molphaProgram.methods
        .updateProtocolConfig({
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: null,
          wormholeBridgeProgram: null,
          targetChainIds: null,
          registryXAgg: null,
          signaturesRequired: signerCount,
          nodeCount: null,
          redundancyBuffer: nodeCount - signerCount,
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      const signersBitmap = Buffer.from(evmUint256BitmapForFirstN(signerCount));

      // Coalition pubkey is the sum of first `signerCount` node pubkeys; coalition privkey is sum of privkeys (mod n).
      let coalitionPoint = secp256k1.Point.ZERO;
      let coalitionSk = 0n;
      for (let i = 0; i < signerCount; i++) {
        coalitionSk = modN(coalitionSk + privKeys[i]);
        coalitionPoint = coalitionPoint.add(secp256k1.Point.BASE.multiply(privKeys[i]));
      }
      const coalitionCompressed = Buffer.from(coalitionPoint.toRawBytes(true)); // 33

      const messageHash = keccakConcat([
        EVM_ROUND_ID_PREFIX,
        jobId,
        u32be(registryVersion),
        u32be(signerCount),
        configHash,
        signersBitmap,
        valueBytes32,
        u64be(timestamp),
      ]);

      const { s, commitmentAddr } = schnorrSignForVerifier({
        coalitionPubkeyCompressed: coalitionCompressed,
        messageHash,
        coalitionPrivkey: coalitionSk,
      });

      const args = {
        jobId: Array.from(jobId),
        value: new BN(valueBytes32.readBigInt64BE(0).toString()),
        valueBytes32: Array.from(valueBytes32),
        timestamp: new BN(timestamp.toString()),
        round: 0,
        signersBitmap: Array.from(signersBitmap),
        s: Array.from(s),
        commitmentAddr: Array.from(commitmentAddr),
        registryVersion,
        configHash: Array.from(configHash),
        maxStaleness: 0,
      };

      const signerNodePdas = nodePdas.slice(0, signerCount);
      const remainingAccounts = signerNodePdas.map((pubkey) => ({
        pubkey,
        isSigner: false,
        isWritable: false,
      }));

      // Create a bankrun-friendly ALT by writing the account directly.
      const banksClient = ctx.provider.context.banksClient;
      const currentSlot = await banksClient.getSlot();
      const [, lutAddress] = AddressLookupTableProgram.createLookupTable({
        authority: ctx.authority.publicKey,
        payer: ctx.authority.publicKey,
        recentSlot: Number(currentSlot),
      });
      const lutData = serializeLookupTableAccountData({
        authority: ctx.authority.publicKey,
        lastExtendedSlot: 0n,
        addresses: signerNodePdas,
      });
      ctx.provider.context.setAccount(lutAddress, {
        lamports: 1_000_000_000n as any,
        executable: false,
        owner: AddressLookupTableProgram.programId,
        data: Uint8Array.from(lutData),
      } as any);
      const lutAccount = new AddressLookupTableAccount({
        key: lutAddress,
        state: {
          deactivationSlot: 0xffff_ffff_ffff_ffffn,
          lastExtendedSlot: 0,
          lastExtendedSlotStartIndex: 0,
          authority: ctx.authority.publicKey,
          addresses: signerNodePdas,
        },
      });

      const verifyIx = await ctx.molphaProgram.methods
        .verifyAnswer(args as any)
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          feed: feedPda,
        })
        .remainingAccounts(remainingAccounts)
        .instruction();

      const bh = await banksClient.getLatestBlockhash();
      assert.exists(bh, "bankrun must provide a recent blockhash");
      const recentBlockhash = bh![0];

      const v0Message = new TransactionMessage({
        payerKey: ctx.authority.publicKey,
        recentBlockhash,
        instructions: [
          ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
          verifyIx,
        ],
      }).compileToV0Message([lutAccount]);

      const vtx = new VersionedTransaction(v0Message);
      vtx.sign([ctx.authority.payer]);

      const res = await banksClient.tryProcessTransaction(vtx as any);
      assert.isNull(res.result, `verify tx must succeed; got error=${res.result}`);
      assert.exists(res.meta, "bankrun must return tx meta");
      return Number(res.meta!.computeUnitsConsumed);
    }

    const cu3 = await measureForSignerCount(3);
    const cu7 = await measureForSignerCount(7);
    const cu18 = await measureForSignerCount(18);
    const cu32 = await measureForSignerCount(32);

    // eslint-disable-next-line no-console
    console.log("verify_answer_alt CU", { cu3, cu7, cu18, cu32 });

    // Conservative ceilings aligned with the optimization doc targets (scaled for bankrun variability).
    assert.isBelow(cu3, 300_000, `3 signers CU too high: ${cu3}`);
    assert.isBelow(cu7, 550_000, `7 signers CU too high: ${cu7}`);
    assert.isBelow(cu18, 950_000, `18 signers CU too high: ${cu18}`);
  });
});

