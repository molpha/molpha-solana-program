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
} from "../setup";

describe("Create Feed Instruction", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  // Test data sources for feed creation tests
  let testDataSources: {
    public: {
      dataSourceType: number;
      source: string;
      name: string;
    };
    private: {
      dataSourceType: number;
      source: string;
      name: string;
    };
  };

  let publicDataSourcePDA: PublicKey;
  let privateDataSourcePDA: PublicKey;

  before(async () => {
    // Generate test data sources with signature
    testDataSources = {
      public: {
        dataSourceType: 0,
        source: "https://api.coinbase.com/v2/prices/BTC-USD/spot",
        name: "Bitcoin Price Public Feed",
      },
      private: {
        dataSourceType: 1,
        source:
          "https://pro-api.coinmarketcap.com/v1/cryptocurrency/quotes/latest",
        name: "CMC Private Bitcoin Feed",
      },
    };

    // Create test data sources for feed creation
    const publicDataSourceInfo = createTestDataSourceInfo(
      testDataSources.public.dataSourceType,
      testDataSources.public.name
    );

    const privateDataSourceInfo = createTestDataSourceInfo(
      testDataSources.private.dataSourceType,
      testDataSources.private.name
    );

    [publicDataSourcePDA] = getDataSourcePda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      testDataSources.public.name,
      testDataSources.public.dataSourceType
    );

    [privateDataSourcePDA] = getDataSourcePda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      testDataSources.private.name,
      testDataSources.private.dataSourceType
    );

    // Create the data sources
    try {
      await ctx.molphaProgram.methods
        .createDataSource(publicDataSourceInfo as any)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          dataSource: publicDataSourcePDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.authority.payer])
        .rpc();
    } catch (e) {
      // Ignore if already exists
    }

    try {
      await ctx.molphaProgram.methods
        .createDataSource(privateDataSourceInfo as any)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          dataSource: privateDataSourcePDA,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.authority.payer])
        .rpc();
    } catch (e) {
      // Ignore if already exists
    }
  });

  it("Successfully creates a public feed with existing data source", async () => {
    const jobId =
      "0xb6c3781295af4f459a731177b25fa4c1a0b4c757152fbbaba977a70f7d635474";
    const feedParams = createFeedParams(jobId, { public: {} });
    const frequencySeed = feedParams.frequency.toBuffer("le", 8);

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(feedParams.name),
        Buffer.from([0]),
        Buffer.from([feedParams.minSignaturesThreshold]),
        feedParams.frequency.toBuffer("le", 8),
        Buffer.from(feedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    // Create feed with 1 day subscription and 1000 lamports priority fee budget
    const subscriptionDurationSeconds = new anchor.BN(86400); // 1 day

    await ctx.molphaProgram.methods
      .createFeed(feedParams, subscriptionDurationSeconds)
      .accountsPartial({
        feed: feedPDA,
        dataSource: publicDataSourcePDA,
        authority: ctx.authority.publicKey,
        protocolConfig: ctx.protocolConfigPDA,
        userTokenAccount: ctx.userTokenAccount,
        programTokenAccount: ctx.programTokenAccount,
        underlyingToken: ctx.underlyingTokenMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .signers([ctx.authority.payer])
      .rpc();

    const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);

    // Verify the feed was created correctly
    assert.ok(feed.authority.equals(ctx.authority.publicKey));
    assert.deepEqual(feed.feedType, { public: {} });
    assert.equal(feed.minSignaturesThreshold, 2);
    assert.equal(feed.frequency.toNumber(), 300);
    assert.equal(feed.ipfsCid, "QmTestCID123456789");
    assert.deepEqual(feed.dataSource, publicDataSourcePDA);
    // assert.deepEqual(
    //   feed.jobId,
    //   Array.from(Buffer.from(jobId.padEnd(32, "\0")))
    // );
  });

  it("Successfully creates a personal feed with existing data source", async () => {
    const jobId = "0x234234";
    const feedParams = createFeedParams(jobId, { personal: {} });

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(feedParams.name),
        Buffer.from([1]),
        Buffer.from([feedParams.minSignaturesThreshold]),
        feedParams.frequency.toBuffer("le", 8),
        Buffer.from(feedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    // Create feed with 1 day subscription and 1000 lamports priority fee budget
    const subscriptionDurationSeconds = new anchor.BN(86400); // 1 day

    await ctx.molphaProgram.methods
      .createFeed(feedParams, subscriptionDurationSeconds)
      .accountsPartial({
        feed: feedPDA,
        dataSource: publicDataSourcePDA,
        authority: ctx.authority.publicKey,
        protocolConfig: ctx.protocolConfigPDA,
        userTokenAccount: ctx.userTokenAccount,
        programTokenAccount: ctx.programTokenAccount,
        underlyingToken: ctx.underlyingTokenMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .signers([ctx.authority.payer])
      .rpc();

    const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);

    // Verify the feed was created correctly
    assert.deepEqual(feed.feedType, { personal: {} });
    assert.equal(feed.minSignaturesThreshold, 2);
  });

  it("Successfully creates a private feed with EthLink", async () => {
    const jobId = "0x22";
    const feedParams = createFeedParams(jobId, { personal: {} });

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(feedParams.name),
        Buffer.from([1]),
        Buffer.from([feedParams.minSignaturesThreshold]),
        feedParams.frequency.toBuffer("le", 8),
        Buffer.from(feedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    await ctx.molphaProgram.methods
      .createFeed(feedParams, new anchor.BN(86400))
      .accountsPartial({
        feed: feedPDA,
        dataSource: privateDataSourcePDA,
        authority: ctx.authority.publicKey,
        protocolConfig: ctx.protocolConfigPDA,
        userTokenAccount: ctx.userTokenAccount,
        programTokenAccount: ctx.programTokenAccount,
        underlyingToken: ctx.underlyingTokenMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .signers([ctx.authority.payer])
      .rpc();

    const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
    assert.deepEqual(feed.dataSource, privateDataSourcePDA);
  });

  it("Fails to create feed with zero min_signatures_threshold", async () => {
    const jobId = "0x1231";

    const invalidParams = createFeedParams(jobId, { public: {} });
    // Override with invalid threshold
    invalidParams.minSignaturesThreshold = 0;

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(invalidParams.name),
        Buffer.from([0]),
        Buffer.from([invalidParams.minSignaturesThreshold]),
        invalidParams.frequency.toBuffer("le", 8),
        Buffer.from(invalidParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    try {
      await ctx.molphaProgram.methods
        .createFeed(invalidParams, new BN(86400))
        .accountsPartial({
          feed: feedPDA,
          dataSource: publicDataSourcePDA,
          authority: ctx.authority.publicKey,
          protocolConfig: ctx.protocolConfigPDA,
          userTokenAccount: ctx.userTokenAccount,
          programTokenAccount: ctx.programTokenAccount,
          underlyingToken: ctx.underlyingTokenMint,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([ctx.authority.payer])
        .rpc();
      assert.fail("Should have failed with zero threshold");
    } catch (error: any) {
      assert.ok(
        error.message.includes("InvalidFeedConfig") ||
          error.message.includes("Invalid feed configuration")
      );
    }
  });

  it("Fails to create feed with empty IPFS CID", async () => {
    const jobId = "0x12312434";

    const invalidParams = createFeedParams(jobId, { public: {} });
    // Override with invalid IPFS CID
    invalidParams.ipfsCid = "";

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(invalidParams.name),
        Buffer.from([0]),
        Buffer.from([invalidParams.minSignaturesThreshold]),
        invalidParams.frequency.toBuffer("le", 8),
        Buffer.from(invalidParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    try {
      await ctx.molphaProgram.methods
        .createFeed(invalidParams, new BN(86400))
        .accountsPartial({
          feed: feedPDA,
          dataSource: publicDataSourcePDA,
          authority: ctx.authority.publicKey,
          protocolConfig: ctx.protocolConfigPDA,
          userTokenAccount: ctx.userTokenAccount,
          programTokenAccount: ctx.programTokenAccount,
          underlyingToken: ctx.underlyingTokenMint,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([ctx.authority.payer])
        .rpc();
      assert.fail("Should have failed with empty IPFS CID");
    } catch (error: any) {
      assert.ok(
        error.message.includes("InvalidFeedConfig") ||
          error.message.includes("Invalid feed configuration")
      );
    }
  });

  it("Fails to create duplicate feed", async () => {
    const jobId =
      "0xb6c3781295af4f459a731177b25fa4c1a0b4c757152fbbaba977a70f7d635474"; // Same as first test
    const feedParams = createFeedParams(jobId, { public: {} });

    const [feedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(feedParams.name),
        Buffer.from([0]),
        Buffer.from([feedParams.minSignaturesThreshold]),
        feedParams.frequency.toBuffer("le", 8),
        Buffer.from(feedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    try {
      await ctx.molphaProgram.methods
        .createFeed(feedParams, new BN(86400))
        .accountsPartial({
          feed: feedPDA,
          dataSource: publicDataSourcePDA,
          authority: ctx.authority.publicKey,
          protocolConfig: ctx.protocolConfigPDA,
          userTokenAccount: ctx.userTokenAccount,
          programTokenAccount: ctx.programTokenAccount,
          underlyingToken: ctx.underlyingTokenMint,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([ctx.authority.payer])
        .rpc();
      assert.fail("Should have failed with duplicate feed");
    } catch (error: any) {
      assert.ok(
        error.message.includes("already in use") ||
          error.message.includes("custom program error") ||
          error.message.includes("Allocate: account")
      );
    }
  });
});
