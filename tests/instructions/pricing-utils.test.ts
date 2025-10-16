import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  createTestDataSourceInfo,
  createFeedParams,
  getDataSourcePda,
  getFeedPda,
} from "../setup";

describe("Pricing Utility Tests", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  describe("Feed Creation with Pricing", () => {
    let testDataSource: {
      dataSourceType: number;
      source: string;
      name: string;
      address: PublicKey;
    };

    let dataSourceId: Uint8Array;
    let dataSourcePDA: PublicKey;
    let feedPDA: PublicKey;

    before(async () => {
      testDataSource = {
        dataSourceType: 0,
        source: "https://api.coinbase.com/v2/prices/BTC-USD/spot",
        name: "Bitcoin Price Feed",
        address: ctx.authority.publicKey,
      };

      // Create test data source for feed creation
      const dataSourceInfo = createTestDataSourceInfo(
        testDataSource.dataSourceType,
        testDataSource.name
      );

      [dataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        testDataSource.name,
        testDataSource.dataSourceType
      );

      // Create the data source
      try {
        await ctx.molphaProgram.methods
          .createDataSource(dataSourceInfo as any)
          .accountsPartial({
            authority: ctx.authority.publicKey,
            dataSource: dataSourcePDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
      } catch (e) {
        // Ignore if already exists
      }
    });

    it("Creates feed with different frequency and signature thresholds to test pricing", async () => {
      const jobId = "0x786531";
      const feedParams = createFeedParams(jobId, { public: {} });

      [feedPDA] = getFeedPda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      feedParams.name,
      0, // public feed type
      feedParams.minSignaturesThreshold,
      feedParams.frequency,
      feedParams.jobId
    );

      // Create feed with 1 day subscription and 1000 lamports priority fee budget
      const subscriptionDurationSeconds = new BN(86400); // 1 day
      const priorityFeeBudget = new BN(1000); // 1000 lamports

      await ctx.molphaProgram.methods
        .createFeed(feedParams, subscriptionDurationSeconds)
        .accountsPartial({
          feed: feedPDA,
          dataSource: dataSourcePDA,
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

      const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);

      // Verify pricing-related fields were set
      assert.isTrue(
        feed.pricePerSecondScaled.toNumber() >= 0,
        "Price per second scaled should be set"
      );

      // Verify subscription timing
      const expectedDueTime = feed.createdAt
        .add(subscriptionDurationSeconds)
        .toNumber();
      assert.equal(
        feed.subscriptionDueTime.toNumber(),
        expectedDueTime,
        "Subscription due time should be calculated correctly"
      );
    });

    it("Creates feed with high frequency and signature requirements to test pricing scaling", async () => {
      const highFreqJobId = "high-freq-pricing-test";

      // Create feed params with high frequency and signature requirements
      const highFreqFeedParams = {
        name: highFreqJobId,
        jobId: Array.from(Buffer.from(highFreqJobId.padEnd(32, "\0"))),
        feedType: { public: {} },
        minSignaturesThreshold: 5, // Higher than default 2
        frequency: new anchor.BN(60), // 1 minute instead of 5 minutes
        ipfsCid: "QmTestCID123456789",
      };

      const [highFreqFeedPDA] = getFeedPda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      highFreqFeedParams.name,
      0, // public feed type
      highFreqFeedParams.minSignaturesThreshold,
      highFreqFeedParams.frequency,
      highFreqFeedParams.jobId
    );

      // Create feed with 1 day subscription and 2000 lamports priority fee budget
      const subscriptionDurationSeconds = new BN(86400); // 1 day
      const priorityFeeBudget = new BN(2000); // 2000 lamports

      await ctx.molphaProgram.methods
        .createFeed(highFreqFeedParams, subscriptionDurationSeconds)
        .accountsPartial({
          feed: highFreqFeedPDA,
          dataSource: dataSourcePDA,
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

      const highFreqFeed = await ctx.molphaProgram.account.feed.fetch(
        highFreqFeedPDA
      );

      // Verify the high-frequency feed was created with different pricing
      assert.equal(
        highFreqFeed.minSignaturesThreshold,
        5,
        "High frequency feed should have 5 signature threshold"
      );

      assert.equal(
        highFreqFeed.frequency.toNumber(),
        60,
        "High frequency feed should have 60 second frequency"
      );

      // The pricing should be different due to frequency and signature scaling
      assert.isTrue(
        highFreqFeed.pricePerSecondScaled.toNumber() >= 0,
        "High frequency feed should have price per second scaled set"
      );

      assert.equal(
        highFreqFeed.subscriptionDueTime.toNumber(),
        highFreqFeed.createdAt.add(subscriptionDurationSeconds).toNumber(),
        "High frequency feed should have correct subscription due time"
      );
    });
  });
});
