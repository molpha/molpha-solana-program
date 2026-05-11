import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";
import {
  initializeProtocol,
  setupTestContext,
  TestContext,
  registerNodeWithDeposit,
  createTestDataSourceInfo,
  getDataSourcePda,
  getFeedPda,
} from "../setup";

function hexToBuf(hex: string): Buffer {
  const normalized = hex.startsWith("0x") ? hex.slice(2) : hex;
  return Buffer.from(normalized, "hex");
}

type VerifyFixture = {
  registeredNodeCount: number;
  registryVersion: number;
  configHash: string;
  nodeIndexesOneBased: number[];
  nodePubkeys: string[];
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

function loadVerifyFixture(): VerifyFixture {
  const p = path.join(__dirname, "..", "fixtures-json", "verify-answer-evm.json");
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

/** EVM `uint256` bitmap: same layout as `evm_bitmap_bit_set` (LSB in byte 31). */
function evmUint256BitmapFromNumber(n: number): number[] {
  const hex = BigInt(n).toString(16).padStart(64, "0");
  return Array.from(Buffer.from(hex, "hex"));
}

function popcountU32(n: number): number {
  let c = 0;
  let x = n >>> 0;
  while (x) {
    c++;
    x &= x - 1;
  }
  return c;
}

/**
 * EVM-style signer bitmap: bit `i` (1-based) selects `nodeIndexesOneBased[i-1]`.
 * Matches `evm_bitmap_bit_set` / `NodeGroupBitmapLib` layout used by the Solidity reference.
 */
function evmSignerOneBasedIndexesFromBitmap(
  bitmap: number,
  maxNodes: number
): number[] {
  const out: number[] = [];
  for (let oneBased = 1; oneBased <= maxNodes; oneBased++) {
    const byteIndex = 31 - ((oneBased - 1) >> 3);
    const bitMask = 1 << ((oneBased - 1) & 7);
    if ((evmUint256BitmapFromNumber(bitmap)[byteIndex] & bitMask) !== 0) {
      out.push(oneBased);
    }
  }
  return out;
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

function serializeLookupTableAccountData(opts: {
  authority: PublicKey;
  lastExtendedSlot: bigint;
  addresses: PublicKey[];
}): Buffer {
  // Matches @solana/web3.js `AddressLookupTableAccount.deserialize` expectations:
  // meta (56 bytes) + addresses (32*N).
  //
  // Meta layout (little endian):
  // - u32 typeIndex (1)
  // - u64 deactivationSlot (u64::MAX means active)
  // - u64 lastExtendedSlot
  // - u8 lastExtendedStartIndex
  // - u8 option (1 == Some(authority))
  // - [32] authority pubkey
  // - [1] padding (to reach 56)
  const U64_MAX = 0xffff_ffff_ffff_ffffn;
  const meta = Buffer.concat([
    u32le(1),
    u64le(U64_MAX),
    u64le(opts.lastExtendedSlot),
    Buffer.from([0x00]),
    Buffer.from([0x01]),
    opts.authority.toBuffer(),
    Buffer.from([0x00, 0x00]),
  ]);
  assert.strictEqual(meta.length, 56, "lookup table meta must be 56 bytes");
  const addrBytes = Buffer.concat(opts.addresses.map((a) => a.toBuffer()));
  return Buffer.concat([meta, addrBytes]);
}

describe("Verify Answer Instruction (ALT / v0 tx)", () => {
  let ctx: TestContext;
  let feedPda: PublicKey;
  const nodePdas: PublicKey[] = [];
  let operatorKeys: Keypair[];
  const VERIFY_FIXTURE = loadVerifyFixture();

  before(async () => {
    assert.strictEqual(VERIFY_FIXTURE.nodePubkeys.length, 10);
    assert.strictEqual(
      VERIFY_FIXTURE.nodePubkeys.length,
      VERIFY_FIXTURE.registeredNodeCount
    );
    assert.strictEqual(
      VERIFY_FIXTURE.dataUpdate.signaturesRequired,
      8,
      "fixture assumes 8-of-10 signing coalition"
    );

    operatorKeys = Array.from({ length: VERIFY_FIXTURE.registeredNodeCount }, () =>
      Keypair.generate()
    );

    ctx = await setupTestContext();
    await initializeProtocol(ctx);

    const dataSourceName = "verify-answer-evm-source-alt";
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
      // already exists in shared bankrun context
    }

    const jobId = hexToBuf(VERIFY_FIXTURE.dataUpdate.jobId);
    [feedPda] = getFeedPda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      "verify-evm-feed-alt",
      0
    );
    await ctx.molphaProgram.methods
      .createFeed(
        {
          name: "verify-evm-feed-alt",
          jobId: Array.from(jobId),
          feedType: { public: {} },
          minSignaturesThreshold: VERIFY_FIXTURE.dataUpdate.signaturesRequired,
          frequency: new BN(300),
          ipfsCid: "QmVerifyVectorAlt",
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
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    const protocolConfigBefore =
      await ctx.molphaProgram.account.protocolConfig.fetch(ctx.protocolConfigPDA);
    const nodeCountBefore = protocolConfigBefore.nodeCount;

    for (let i = 0; i < VERIFY_FIXTURE.registeredNodeCount; i++) {
      const nodePda = await registerNodeWithDeposit(
        ctx,
        operatorKeys[i].publicKey,
        hexToBuf(VERIFY_FIXTURE.nodePubkeys[i])
      );
      nodePdas.push(nodePda);
    }

    const protocolConfig =
      await ctx.molphaProgram.account.protocolConfig.fetch(ctx.protocolConfigPDA);
    assert.strictEqual(
      protocolConfig.nodeCount,
      nodeCountBefore + VERIFY_FIXTURE.registeredNodeCount,
      "register_node must increment node_count once per registration (works with shared bankrun)"
    );

    const nodeCount = Number(protocolConfig.nodeCount);
    const sigReq = VERIFY_FIXTURE.dataUpdate.signaturesRequired;
    assert.isAtLeast(
      nodeCount,
      sigReq,
      `fixture needs node_count >= signatures_required (${sigReq}); got ${nodeCount}`
    );

    // Ensure deterministic group bitmap: group_size == node_count (full bitmap).
    const redundancyForFullGroup = nodeCount - sigReq;
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
        signaturesRequired: sigReq,
        nodeCount: null,
        redundancyBuffer: redundancyForFullGroup,
      })
      .accountsPartial({
        protocolConfig: ctx.protocolConfigPDA,
        authority: ctx.authority.publicKey,
      })
      .rpc();
  });

  it("verifies using a v0 tx whose signer Node PDAs come from an ALT", async () => {
    const valueBytes = hexToBuf(VERIFY_FIXTURE.dataUpdate.value);
    const signersBitmap = evmUint256BitmapFromNumber(
      VERIFY_FIXTURE.schnorrSignature.signersBitmap
    );

    const commitmentAddr = hexToBuf(VERIFY_FIXTURE.schnorrSignature.commitment);
    assert.strictEqual(commitmentAddr.length, 20, "fixture commitment must be 20 bytes");

    const args = {
      jobId: Array.from(hexToBuf(VERIFY_FIXTURE.dataUpdate.jobId)),
      value: new BN(valueBytes.readBigInt64BE(0).toString()),
      valueBytes32: Array.from(valueBytes),
      timestamp: new BN(VERIFY_FIXTURE.dataUpdate.canonicalTimestamp),
      round: 0,
      signersBitmap,
      s: Array.from(hexToBuf(VERIFY_FIXTURE.schnorrSignature.signature)),
      commitmentAddr: Array.from(commitmentAddr),
      registryVersion: VERIFY_FIXTURE.dataUpdate.registryVersion,
      configHash: Array.from(hexToBuf(VERIFY_FIXTURE.dataUpdate.configHash)),
      maxStaleness: 0,
    };

    const signerOneBased = evmSignerOneBasedIndexesFromBitmap(
      VERIFY_FIXTURE.schnorrSignature.signersBitmap,
      VERIFY_FIXTURE.registeredNodeCount
    );
    assert.strictEqual(
      signerOneBased.length,
      popcountU32(VERIFY_FIXTURE.schnorrSignature.signersBitmap),
      "signer set size must match bitmap popcount"
    );

    const signerNodePdas = signerOneBased.map((idx) => nodePdas[idx - 1]);

    // ---- Create an ALT (bankrun-friendly) containing signer Node PDAs ----
    const client = ctx.provider.context.banksClient;
    // `AddressLookupTableProgram.createLookupTable` enforces a \"recentSlot\" constraint
    // against SlotHashes, which is brittle in bankrun. Instead, we directly create the
    // lookup table account with valid serialized data via `context.setAccount`.
    const recentSlot = await client.getSlot();
    const [, lutAddress] = AddressLookupTableProgram.createLookupTable({
      authority: ctx.authority.publicKey,
      payer: ctx.authority.publicKey,
      recentSlot: Number(recentSlot),
    });

    const lutData = serializeLookupTableAccountData({
      authority: ctx.authority.publicKey,
      // Make all addresses immediately active: if `currentSlot == lastExtendedSlot`,
      // only the prefix `[0..lastExtendedSlotStartIndex)` is active. Setting this to
      // a past slot avoids \"recent extension\" restrictions.
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

    // ---- Build verify ix and compile to v0 message using ALT ----
    const remainingAccounts = signerNodePdas.map((pubkey) => ({
      pubkey,
      isSigner: false,
      isWritable: false,
    }));

    const verifyIx = await ctx.molphaProgram.methods
      .verifyAnswer(args as any)
      .accountsPartial({
        protocolConfig: ctx.protocolConfigPDA,
        feed: feedPda,
      })
      .remainingAccounts(remainingAccounts)
      .instruction();

    const bh = await client.getLatestBlockhash();
    assert.exists(bh, "bankrun must provide a recent blockhash");
    const recentBlockhash = bh![0];

    const v0Message = new TransactionMessage({
      payerKey: ctx.authority.publicKey,
      recentBlockhash,
      instructions: [
        ComputeBudgetProgram.setComputeUnitLimit({ units: 500_000 }),
        verifyIx,
      ],
    }).compileToV0Message([lutAccount]);

    // Strong guarantee: compiled message must actually use the LUT.
    assert.isAtLeast(
      v0Message.addressTableLookups.length,
      1,
      "v0 message must include address table lookups"
    );
    assert.strictEqual(
      v0Message.addressTableLookups[0].accountKey.toBase58(),
      lutAddress.toBase58(),
      "v0 message must reference the created LUT"
    );

    const staticKeys = v0Message.staticAccountKeys.map((k) => k.toBase58());
    const atLeastOneSignerNotStatic = signerNodePdas.some(
      (k) => !staticKeys.includes(k.toBase58())
    );
    assert.isTrue(
      atLeastOneSignerNotStatic,
      "at least one signer Node PDA must be resolved via ALT (not a static account key)"
    );

    const vtx = new VersionedTransaction(v0Message);
    vtx.sign([ctx.authority.payer]);

    const res = await client.tryProcessTransaction(vtx as any);
    assert.isNull(res.result, `verify tx must succeed; got error=${res.result}`);
  });
});

