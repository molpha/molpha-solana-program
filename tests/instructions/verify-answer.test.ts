import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as secp256k1 from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
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

function commitmentAddrFromRxParity(rx: Buffer, ryParity: number): Buffer {
  const prefix = ryParity === 0 ? 0x02 : 0x03;
  const compressed = Buffer.concat([Buffer.from([prefix]), rx]);
  const R = secp256k1.Point.fromHex(Uint8Array.from(compressed));
  const uncompressed = Buffer.from(R.toRawBytes(false)); // 65: 0x04 || x || y
  const digest = Buffer.from(keccak_256(uncompressed.subarray(1))); // keccak(x||y)
  return digest.subarray(12, 32); // last 20 bytes
}

/**
 * Cross-chain verify vector (`verify_answer` preimage matches `Validator._constructMessage`:
 * registryVersion, configHash, signersBitmap, etc. — see `programs/molpha/src/instructions/verify_answer.rs`).
 */
const VERIFY_FIXTURE = {
  keysArrayLength: 6,
  registeredNodeCount: 5,
  nodeIndexesOneBased: [1, 2, 3, 4, 5],
  nodePubkeys: [
    "0x03c09527e978f6ea69f0c6b7ac0fb63ad081a8a291151c5a0b115cce435751be7d",
    "0x0264a72704f39f8dd17f20d71c5b21f37b5852656bc0555442bf727222f29d7e58",
    "0x0275ae1e3dac00eb7df02e9fe8d9709c8a2c09a11ed4f7d9aa46a7dea6cf376d7f",
    "0x026ce25b3a161ab8e0f05e4cd1c77b77696d26c641ebdea4e81aa89a90f32cfc54",
    "0x035b95d703228befccc378629dc19804cefe56c33c645fa4bc1aa0f375e3b4fa5e",
  ],
  dataUpdate: {
    jobId:
      "0x6a6f622d65787472616374000000000000000000000000000000000000000000",
    registryVersion: 42,
    signaturesRequired: 3,
    configHash:
      "0x6366672d65787472616374000000000000000000000000000000000000000000",
    value: "0x76616c2d65787472616374000000000000000000000000000000000000000000",
    canonicalTimestamp: 1700001234,
  },
  schnorrSignature: {
    s: "0x367288bf1bbda4cd584479320c9e96a2fbe86f76a88463c36ca827030167c951",
    rx: "0x5866b9f352c60f7f9334f09056885e2281421a79de81a7127ce2b2e6e2b9063c",
    ryParity: 1,
    signersBitmap: 7,
  },
} as const;

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

describe("Verify Answer Instruction - EVM vector compatibility", () => {
  let ctx: TestContext;
  let feedPda: PublicKey;
  const nodePdas: PublicKey[] = [];
  /** Six Solana operator keys: first `registeredNodeCount` are registered; sixth is unused (EVM keys-array padding). */
  let operatorKeys: Keypair[];

  before(async () => {
    assert.strictEqual(
      VERIFY_FIXTURE.nodePubkeys.length,
      VERIFY_FIXTURE.registeredNodeCount
    );
    assert.strictEqual(
      VERIFY_FIXTURE.nodeIndexesOneBased.length,
      VERIFY_FIXTURE.registeredNodeCount
    );

    operatorKeys = Array.from({ length: VERIFY_FIXTURE.keysArrayLength }, () =>
      Keypair.generate()
    );

    ctx = await setupTestContext();
    await initializeProtocol(ctx);

    const dataSourceName = "verify-answer-evm-source";
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
      "verify-evm-feed",
      0
    );
    await ctx.molphaProgram.methods
      .createFeed(
        {
          name: "verify-evm-feed",
          jobId: Array.from(jobId),
          feedType: { public: {} },
          minSignaturesThreshold: VERIFY_FIXTURE.dataUpdate.signaturesRequired,
          frequency: new BN(300),
          ipfsCid: "QmVerifyVector",
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
    /**
     * `derive_group_bitmap` matches EVM only when `group_size == node_count`
     * (full-set bitmap). That requires `signatures_required + redundancy_buffer == node_count`.
     * With a **fresh** ledger that is exactly `registeredNodeCount`, this is 3 + 2 == 5.
     * When Jest runs many files in one process, bankrun can accumulate nodes on the same
     * genesis — then a fixed `redundancyBuffer: 2` leaves `group_size < node_count` and
     * selection becomes random → flaky `SignerNotInGroup` vs bitmap `7` (nodes 1–3).
     */
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

    const cfgCheck =
      await ctx.molphaProgram.account.protocolConfig.fetch(ctx.protocolConfigPDA);
    assert.strictEqual(
      Number(cfgCheck.signaturesRequired),
      sigReq,
      "signatures_required must be applied for deterministic group derivation"
    );
    assert.strictEqual(
      Number(cfgCheck.redundancyBuffer),
      redundancyForFullGroup,
      "redundancy_buffer must make group_size == node_count (full bitmap)"
    );

    // Debug: coalition aggregate for the fixture signer bitmap (NOT the sum of all registered nodes).
    const signerOneBased = evmSignerOneBasedIndexesFromBitmap(
      VERIFY_FIXTURE.schnorrSignature.signersBitmap,
      VERIFY_FIXTURE.registeredNodeCount
    );
    assert.strictEqual(
      signerOneBased.length,
      popcountU32(VERIFY_FIXTURE.schnorrSignature.signersBitmap)
    );

    let agg = secp256k1.Point.fromHex(
      Uint8Array.from(hexToBuf(VERIFY_FIXTURE.nodePubkeys[signerOneBased[0] - 1]))
    );
    for (let k = 1; k < signerOneBased.length; k++) {
      agg = agg.add(
        secp256k1.Point.fromHex(
          Uint8Array.from(
            hexToBuf(VERIFY_FIXTURE.nodePubkeys[signerOneBased[k] - 1])
          )
        )
      );
    }
    const expectedAggComp = Buffer.from(agg.toRawBytes(true));

    const node0 = await ctx.molphaProgram.account.node.fetch(
      nodePdas[signerOneBased[0] - 1]
    );
    let onchainAgg = secp256k1.Point.fromHex(
      Uint8Array.from(
        Buffer.concat([
          Buffer.from([0x04]),
          Buffer.from(node0.nodePubEvmX),
          Buffer.from(node0.nodePubEvmY),
        ])
      )
    );
    for (let k = 1; k < signerOneBased.length; k++) {
      const nodeK = await ctx.molphaProgram.account.node.fetch(
        nodePdas[signerOneBased[k] - 1]
      );
      const uncompressed = Buffer.concat([
        Buffer.from([0x04]),
        Buffer.from(nodeK.nodePubEvmX),
        Buffer.from(nodeK.nodePubEvmY),
      ]);
      onchainAgg = onchainAgg.add(
        secp256k1.Point.fromHex(Uint8Array.from(uncompressed))
      );
    }
    const onchainAggComp = Buffer.from(onchainAgg.toRawBytes(true));

    assert.strictEqual(
      onchainAggComp.toString("hex"),
      expectedAggComp.toString("hex"),
      "on-chain node_pub_evm aggregate must match fixture coalition pubkeys"
    );
  });

  /**
   * End-to-end cross-chain compatibility vector:
   * - group selection + EVM-style bitmap layout
   * - `_constructMessage` preimage (see `ref/src/Validator.sol`)
   * - Schnorr verify via the EVM `ecrecover` construction (ported on Solana via `secp256k1_recover`)
   *
   * On-chain, `remaining_accounts` order does not affect verification (program sorts by `node_index`).
   * We still pass Node PDAs in bitmap bit order (nodes 1..k) for readability.
   */
  it("walks the EVM vector through bitmap/group checks and verifies the signature", async () => {
    const valueBytes = hexToBuf(VERIFY_FIXTURE.dataUpdate.value);
    const signersBitmap = evmUint256BitmapFromNumber(
      VERIFY_FIXTURE.schnorrSignature.signersBitmap
    );

    const args = {
      jobId: Array.from(hexToBuf(VERIFY_FIXTURE.dataUpdate.jobId)),
      value: new BN(valueBytes.readBigInt64BE(0).toString()),
      valueBytes32: Array.from(valueBytes),
      timestamp: new BN(VERIFY_FIXTURE.dataUpdate.canonicalTimestamp),
      round: 0,
      signersBitmap,
      s: Array.from(hexToBuf(VERIFY_FIXTURE.schnorrSignature.s)),
      commitmentAddr: Array.from(
        commitmentAddrFromRxParity(
          hexToBuf(VERIFY_FIXTURE.schnorrSignature.rx),
          VERIFY_FIXTURE.schnorrSignature.ryParity
        )
      ),
      registryVersion: VERIFY_FIXTURE.dataUpdate.registryVersion,
      configHash: Array.from(hexToBuf(VERIFY_FIXTURE.dataUpdate.configHash)),
      maxStaleness: 0,
    };

    assert.strictEqual(
      Buffer.from(args.s).toString("hex"),
      VERIFY_FIXTURE.schnorrSignature.s.replace(/^0x/, ""),
      "sanity: ix args.s must match fixture hex"
    );

    const signerOneBased = evmSignerOneBasedIndexesFromBitmap(
      VERIFY_FIXTURE.schnorrSignature.signersBitmap,
      VERIFY_FIXTURE.registeredNodeCount
    );
    assert.strictEqual(
      signerOneBased.length,
      popcountU32(VERIFY_FIXTURE.schnorrSignature.signersBitmap),
      "remaining_accounts count must match bitmap popcount"
    );

    const remainingAccounts = signerOneBased.map((idx) => ({
      pubkey: nodePdas[idx - 1],
      isSigner: false,
      isWritable: false,
    }));

    await ctx.molphaProgram.methods
      .verifyAnswer(args as any)
      .accountsPartial({
        protocolConfig: ctx.protocolConfigPDA,
        feed: feedPda,
      })
      .preInstructions([
        // This instruction can exceed 800k CUs on bankrun when PDA fallback runs per signer; keep ample budget.
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .remainingAccounts(remainingAccounts)
      .rpc();
  });
});
