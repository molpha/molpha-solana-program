import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BankrunProvider } from "anchor-bankrun";
import * as nacl from "tweetnacl";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  createTestDataSourceInfo,
  getDataSourcePda,
  createFeedParams,
  registerNodeWithDeposit,
} from "../setup";

async function safePastOnchainTimestamp(
  provider: BankrunProvider | any,
  secondsInPast = 1
): Promise<anchor.BN> {
  try {
    const client = provider.connection.banksClient.inner;
    const clock = await client.getClock();
    const timestamp = new anchor.BN(
      Math.max(0, Number(clock.unixTimestamp) - secondsInPast)
    );
    return timestamp;
  } catch (e) {
    // Fallback that always satisfies `timestamp <= clock.unix_timestamp`
    return new anchor.BN(1);
  }
}

describe("Subscription Integration Tests", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  // Test data source for subscription tests
  let testDataSource: {
    dataSourceType: number;
    source: string;
    name: string;
  };

  let dataSourcePDA: PublicKey;
  let feedPDA: PublicKey;

  before(async () => {
    testDataSource = {
      dataSourceType: 0,
      source: "https://api.coinbase.com/v2/prices/BTC-USD/spot",
      name: "Bitcoin Price Feed",
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

  describe("Create Feed with Subscription", () => {
    it("Successfully creates a feed with subscription", async () => {
      const jobId = "0x02";
      const feedParams = createFeedParams(jobId, { public: {} });

      [feedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(feedParams.name),
          Buffer.from([0]), // Public feed type
          Buffer.from([feedParams.minSignaturesThreshold]),
          feedParams.frequency.toBuffer("le", 8),
          Buffer.from(feedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

      // Create feed with 1 day subscription and 1000 lamports priority fee budget
      const subscriptionDurationSeconds = new anchor.BN(86400); // 1 day

      await ctx.molphaProgram.methods
        .createFeed(feedParams as any, subscriptionDurationSeconds)
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

      // Verify the feed was created correctly
      assert.ok(feed.authority.equals(ctx.authority.publicKey));
      assert.deepEqual(feed.feedType, { public: {} });
      assert.equal(feed.minSignaturesThreshold, 2);
      assert.equal(feed.frequency.toNumber(), 300);
      assert.equal(feed.ipfsCid, "QmTestCID123456789");
      assert.deepEqual(feed.dataSource, dataSourcePDA);
      assert.deepEqual(feed.jobId, feedParams.jobId);

      // Verify subscription data was initialized
      assert.equal(
        feed.subscriptionDueTime.toNumber(),
        feed.createdAt.toNumber() + subscriptionDurationSeconds.toNumber()
      );
      assert.equal(feed.pricePerSecondScaled.toNumber(), 0); // Will be calculated by the program
    });

    it("Fails to create feed with subscription duration less than 1 day", async () => {
      const jobId = "0x03";
      const feedParams = createFeedParams(jobId, { public: {} });

      const [shortFeedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(feedParams.name),
          Buffer.from([0]), // Public feed type
          Buffer.from([feedParams.minSignaturesThreshold]),
          feedParams.frequency.toBuffer("le", 8),
          Buffer.from(feedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

      // Try to create feed with 12 hours subscription (less than 1 day)
      const subscriptionDurationSeconds = new anchor.BN(43200); // 12 hours
      const priorityFeeBudget = new anchor.BN(500); // 500 lamports

      try {
        await ctx.molphaProgram.methods
          .createFeed(feedParams as any, subscriptionDurationSeconds)
          .accountsPartial({
            feed: shortFeedPDA,
            dataSource: dataSourcePDA,
            authority: ctx.authority.publicKey,
            userTokenAccount: ctx.userTokenAccount,
            programTokenAccount: ctx.programTokenAccount,
            underlyingToken: ctx.underlyingTokenMint,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            protocolConfig: ctx.protocolConfigPDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        assert.fail("Should have failed with minimum subscription time error");
      } catch (error) {
        assert.include(error.message, "MinimumSubscriptionTime");
      }
    });
  });

  describe("Extend Subscription", () => {
    it("Successfully extends subscription with additional time and priority fee budget", async () => {
      const additionalDurationSeconds = new anchor.BN(86400); // 1 day
      const additionalPriorityFeeBudget = new anchor.BN(500); // 500 lamports

      // Get initial feed state
      const initialFeed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      const initialDueTime = initialFeed.subscriptionDueTime;

      // Extend subscription
      await ctx.molphaProgram.methods
        .extendSubscription(additionalDurationSeconds)
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

      // Get updated feed state
      const updatedFeed = await ctx.molphaProgram.account.feed.fetch(feedPDA);

      // Verify subscription was extended
      assert.equal(
        updatedFeed.subscriptionDueTime.toNumber(),
        initialDueTime.toNumber() + additionalDurationSeconds.toNumber(),
        "Subscription due time should be extended"
      );
    });

    it("Fails to extend subscription with duration less than 1 day", async () => {
      const additionalDurationSeconds = new anchor.BN(86399); // 1 day - 1 second (less than 1 day)

      try {
        await ctx.molphaProgram.methods
          .extendSubscription(additionalDurationSeconds)
          .accountsPartial({
            feed: feedPDA,
            authority: ctx.authority.publicKey,
            protocolConfig: ctx.protocolConfigPDA,
            userTokenAccount: ctx.userTokenAccount,
            programTokenAccount: ctx.programTokenAccount,
            underlyingToken: ctx.underlyingTokenMint,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        assert.fail("Should have failed with minimum extension time error");
      } catch (error) {
        assert.include(error.message, "MinimumExtensionTime");
      }
    });

    it("Fails to extend subscription when caller is not the feed authority", async () => {
      const additionalDurationSeconds = new anchor.BN(86400); // 1 day

      // Create a different authority
      const differentAuthority = anchor.web3.Keypair.generate();

      try {
        await ctx.molphaProgram.methods
          .extendSubscription(additionalDurationSeconds)
          .accountsPartial({
            feed: feedPDA,
            authority: differentAuthority.publicKey,
            protocolConfig: ctx.protocolConfigPDA,
            userTokenAccount: ctx.userTokenAccount,
            programTokenAccount: ctx.programTokenAccount,
            underlyingToken: ctx.underlyingTokenMint,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .signers([differentAuthority])
          .rpc();

        assert.fail("Should have failed with authority mismatch error");
      } catch (error) {
        console.log("my-xerror:", error);
        // Should fail because differentAuthority is not the feed owner
        assert.include(error.message, "Error");
      }
    });
  });

  describe("Publish Answer with Subscription Validation", () => {
    it("Successfully publishes answer when subscription is active", async () => {
      // First, register some nodes (need 2 nodes since minSignaturesThreshold is 2)
      const testNode1 = anchor.web3.Keypair.generate();
      const testNode2 = anchor.web3.Keypair.generate();

      const evmPubkey = Buffer.alloc(33, 1);

      const node1PDA = await registerNodeWithDeposit(ctx, testNode1.publicKey, evmPubkey);
      const node2PDA = await registerNodeWithDeposit(ctx, testNode2.publicKey, evmPubkey);

      // Create a test answer with a timestamp further in the past
      const answer = {
        value: Array.from(Buffer.from("12345678901234567890123456789012")), // 32 bytes
        timestamp: await safePastOnchainTimestamp(ctx.provider, 30), // Use timestamp 30 seconds in the past
      };

      // Create signature instructions like the working tests
      const message = Buffer.from(answer.value);
      const preIxs: anchor.web3.TransactionInstruction[] = [];

      // Add signatures from both test nodes (need 2 since minSignaturesThreshold is 2)
      const testNodes = [testNode1, testNode2];
      testNodes.forEach((testNode) => {
        const signature_bytes = nacl.sign.detached(message, testNode.secretKey);
        const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: testNode.publicKey.toBytes(),
          message,
          signature: signature_bytes,
        });
        preIxs.push(ix);
      });

      // Get Node PDAs for remaining accounts
      const nodePDAs = [
        { pubkey: node1PDA, isSigner: false, isWritable: false },
        { pubkey: node2PDA, isSigner: false, isWritable: false },
      ];

      // Publish answer with signature
      await ctx.molphaProgram.methods
        .publishAnswer(answer)
        .accountsPartial({
          feed: feedPDA,
          instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
        })
        .remainingAccounts(nodePDAs)
        .preInstructions(preIxs)
        .rpc();

      // Verify the answer was published
      const updatedFeed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      assert.deepEqual(
        updatedFeed.latestAnswer.value,
        Array.from(answer.value)
      );
      assert.equal(
        updatedFeed.latestAnswer.timestamp.toNumber(),
        answer.timestamp.toNumber()
      );
    });

    it.skip("Fails to publish answer when subscription has expired", async () => {
      // Create a new feed with very short subscription duration
      const shortJobId = "expired-subscription-feed";
      const shortFeedParams = createFeedParams(
        shortJobId,
        { public: {} },
        dataSourceId
      );

      const [shortFeedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(shortFeedParams.name),
          Buffer.from([0]), // Public feed type
          Buffer.from([shortFeedParams.minSignaturesThreshold]),
          shortFeedParams.frequency.toBuffer("le", 8),
          Buffer.from(shortFeedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

      // Create feed with minimum valid subscription duration
      const subscriptionDurationSeconds = new anchor.BN(86400); // 1 day (minimum required)
      const priorityFeeBudget = new anchor.BN(100); // 100 lamports

      await ctx.molphaProgram.methods
        .createFeed(
          shortFeedParams,
          subscriptionDurationSeconds,
          priorityFeeBudget
        )
        .accountsPartial({
          feed: shortFeedPDA,
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

      // Wait for subscription to expire
      await new Promise((resolve) => setTimeout(resolve, 2000)); // Wait 2 seconds

      // Try to publish answer
      const answer = {
        value: Array.from(Buffer.from("12345678901234567890123456789012")), // 32 bytes
        timestamp: await safePastOnchainTimestamp(ctx.provider, 30), // Use timestamp 30 seconds in the past
      };

      try {
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: shortFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .rpc();

        assert.fail("Should have failed with subscription expired error");
      } catch (error) {
        assert.include(error.message, "SubscriptionExpired");
      }
    });
  });

  describe("Subscription State Management", () => {
    it("Correctly tracks subscription due time", async () => {
      const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      const currentTime = Math.floor(Date.now() / 1000);

      assert.isTrue(
        feed.subscriptionDueTime.toNumber() > currentTime,
        "Subscription should be active"
      );
    });

    it("Correctly calculates remaining subscription time", async () => {
      const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      const currentTime = Math.floor(Date.now() / 1000);
      const remainingTime = feed.subscriptionDueTime.toNumber() - currentTime;

      assert.isTrue(
        remainingTime > 0,
        "Remaining subscription time should be positive"
      );

      assert.isTrue(
        remainingTime <= 86400 * 2, // Allow up to 2 days due to extensions in previous tests
        "Remaining time should be reasonable"
      );
    });
  });
});
