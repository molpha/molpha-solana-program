import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import { assert, expect } from "chai";
import {
  PublicKey,
  SystemProgram,
  Keypair,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BankrunProvider, startAnchor, Clock } from "anchor-bankrun";
import * as fs from "fs";
import * as toml from "toml";
import { Molpha } from "../target/types/molpha";
import { FeedConsumer } from "../target/types/feed_consumer";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  registerNodeWithDeposit,
  createTestDataSourceInfo,
  getDataSourcePda,
  getFeedPda,
} from "./setup";
import nacl from "tweetnacl";

interface FeedConsumerTestContext {
  molphaProgram: Program<Molpha>;
  feedConsumerProgram: Program<FeedConsumer>;
  provider: BankrunProvider;
  authority: anchor.Wallet;
  feedPDA: PublicKey;
  dataSourcePDA: PublicKey;
  nodePDA: PublicKey;
  nodeKeypair: Keypair;
}

async function setupFeedConsumerTestContext(): Promise<FeedConsumerTestContext> {
  // First setup the base Molpha context
  const molphaCtx = await setupTestContext();
  await initializeProtocol(molphaCtx);

  // Get FeedConsumer program
  const feedConsumerProgram = anchor.workspace
    .FeedConsumer as Program<FeedConsumer>;

  // Register a node for publishing answers
  const nodeKeypair = Keypair.generate();
  const evmPubkey = Buffer.alloc(33, 1);
  const nodePDA = await registerNodeWithDeposxit(
    molphaCtx,
    nodeKeypair.publicKey,
    evmPubkey,
    "https://test-node.com"
  );

  // Create a data source
  const dataSourceName = "BTC/USD Price Feed";
  const dataSourceInfo = createTestDataSourceInfo(0, dataSourceName);
  const [dataSourcePDA] = getDataSourcePda(
    molphaCtx.molphaProgram.programId,
    molphaCtx.authority.publicKey,
    dataSourceName,
    0
  );

  try {
    await molphaCtx.molphaProgram.methods
      .createDataSource(dataSourceInfo as any)
      .accountsPartial({
        dataSource: dataSourcePDA,
        authority: molphaCtx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  } catch (e) {
    // Ignore if already exists
  }

  // Create a public feed
  const feedName = "btc-usd-feed";
  const feedParams = {
    jobId: Array.from(Buffer.from(feedName.padEnd(32, "\0"))),
    feedType: { public: {} },
    minSignaturesThreshold: 1, // Only need 1 signature for testing
    frequency: new anchor.BN(60), // 60 seconds
    ipfsCid: "QmTestFeed",
    name: feedName,
  };

  const [feedPDA] = getFeedPda(
    molphaCtx.molphaProgram.programId,
    molphaCtx.authority.publicKey,
    feedParams.name,
    0 // public feed type
  );

  try {
    await molphaCtx.molphaProgram.methods
      .createFeed(feedParams, new anchor.BN(86400)) // 24 hours subscription
      .accountsPartial({
        feed: feedPDA,
        dataSource: dataSourcePDA,
        underlyingToken: molphaCtx.underlyingTokenMint,
        userTokenAccount: molphaCtx.userTokenAccount,
        programTokenAccount: molphaCtx.programTokenAccount,
        protocolConfig: molphaCtx.protocolConfigPDA,
        authority: molphaCtx.authority.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
  } catch (e) {
    // Ignore if already exists
  }

  return {
    molphaProgram: molphaCtx.molphaProgram,
    feedConsumerProgram,
    provider: molphaCtx.provider,
    authority: molphaCtx.authority,
    feedPDA,
    dataSourcePDA,
    nodePDA,
    nodeKeypair,
  };
}

async function publishAnswerToFeed(
  ctx: FeedConsumerTestContext,
  price: number,
  conf: number,
  expo: number
): Promise<void> {
  const clock = await ctx.provider.context.banksClient.getClock();
  const timestamp = Number(clock.unixTimestamp);

  // Encode price data into Answer.value format
  // Layout: [price: i64][conf: u64][expo: i32][reserved: 12 bytes]
  const value = Buffer.alloc(32);
  value.writeBigInt64LE(BigInt(price), 0);
  value.writeBigUInt64LE(BigInt(conf), 8);
  value.writeInt32LE(expo, 16);

  const answer = {
    value: Array.from(value),
    timestamp: new BN(timestamp),
  };

  // Create message for signing
  const messageBuffer = Buffer.alloc(40);
  value.copy(messageBuffer, 0, 0, 32);
  messageBuffer.writeBigInt64LE(BigInt(timestamp), 32);

  // Sign with node's keypair
  const signature = nacl.sign.detached(
    messageBuffer,
    ctx.nodeKeypair.secretKey
  );

  await ctx.molphaProgram.methods
    .publishAnswer(answer)
    .accountsPartial({
      feed: ctx.feedPDA,
      node: ctx.nodePDA,
      authority: ctx.authority.publicKey,
    })
    .remainingAccounts([
      {
        pubkey: ctx.nodeKeypair.publicKey,
        isSigner: false,
        isWritable: false,
      },
      {
        pubkey: ctx.nodeKeypair.publicKey,
        isSigner: false,
        isWritable: false,
      },
    ])
    .signers([])
    .rpc();
}

describe("Feed Consumer Program", () => {
  let ctx: FeedConsumerTestContext;
  let snapshotPDA: PublicKey;

  before(async () => {
    ctx = await setupFeedConsumerTestContext();

    // Derive the snapshot PDA
    [snapshotPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("snapshot"),
        ctx.authority.publicKey.toBuffer(),
        ctx.feedPDA.toBuffer(),
      ],
      ctx.feedConsumerProgram.programId
    );
  });

  describe("Store Snapshot", () => {
    xit("Successfully stores a snapshot from a valid feed", async () => {
      // First, publish an answer to the feed
      const price = 50000_00000000; // $50,000 with -8 exponent
      const conf = 10_00000000; // $10 confidence
      const expo = -8;

      await publishAnswerToFeed(ctx, price, conf, expo);

      // Now store the snapshot
      await ctx.feedConsumerProgram.methods
        .storeSnapshot(null) // No max_age restriction
        .accountsPartial({
          authority: ctx.authority.publicKey,
          feed: ctx.feedPDA,
          snapshot: snapshotPDA,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      // Verify the snapshot was created correctly
      const snapshot = await ctx.feedConsumerProgram.account.snapshot.fetch(
        snapshotPDA
      );

      assert.ok(snapshot.authority.equals(ctx.authority.publicKey));
      assert.ok(snapshot.feed.equals(ctx.feedPDA));

      // Verify the price data was copied correctly
      const valueBuffer = Buffer.from(snapshot.value);
      const storedPrice = valueBuffer.readBigInt64LE(0);
      const storedConf = valueBuffer.readBigUInt64LE(8);
      const storedExpo = valueBuffer.readInt32LE(16);

      assert.equal(storedPrice.toString(), BigInt(price).toString());
      assert.equal(storedConf.toString(), BigInt(conf).toString());
      assert.equal(storedExpo, expo);

      assert.isTrue(snapshot.timestamp.toNumber() > 0);
      assert.isTrue(snapshot.storedAt.toNumber() > 0);

      console.log("✓ Snapshot stored successfully");
      console.log(`  Price: ${storedPrice.toString()}`);
      console.log(`  Confidence: ${storedConf.toString()}`);
      console.log(`  Exponent: ${storedExpo}`);
      console.log(`  Timestamp: ${snapshot.timestamp.toString()}`);
    });

    xit("Successfully updates an existing snapshot with new feed data", async () => {
      // Get the initial snapshot
      const initialSnapshot =
        await ctx.feedConsumerProgram.account.snapshot.fetch(snapshotPDA);
      const initialStoredAt = initialSnapshot.storedAt.toNumber();

      // Advance clock by 70 seconds
      const currentClock = await ctx.provider.context.banksClient.getClock();
      const newClock = new Clock(
        currentClock.slot + BigInt(70),
        currentClock.epochStartTimestamp,
        currentClock.epoch,
        currentClock.leaderScheduleEpoch,
        currentClock.unixTimestamp + BigInt(70)
      );
      ctx.provider.context.setClock(newClock);

      // Publish a new answer with updated price
      const newPrice = 51000_00000000; // $51,000
      const conf = 10_00000000;
      const expo = -8;

      await publishAnswerToFeed(ctx, newPrice, conf, expo);

      // Update the snapshot
      await ctx.feedConsumerProgram.methods
        .storeSnapshot(null)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          feed: ctx.feedPDA,
          snapshot: snapshotPDA,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      // Verify the snapshot was updated
      const updatedSnapshot =
        await ctx.feedConsumerProgram.account.snapshot.fetch(snapshotPDA);

      const valueBuffer = Buffer.from(updatedSnapshot.value);
      const storedPrice = valueBuffer.readBigInt64LE(0);

      assert.equal(storedPrice.toString(), BigInt(newPrice).toString());
      assert.isTrue(updatedSnapshot.storedAt.toNumber() > initialStoredAt);

      console.log("✓ Snapshot updated successfully");
      console.log(`  New Price: ${storedPrice.toString()}`);
      console.log(`  Previous stored_at: ${initialStoredAt}`);
      console.log(`  New stored_at: ${updatedSnapshot.storedAt.toString()}`);
    });

    xit("Rejects stale feed data when max_age is specified", async () => {
      // Advance clock by 200 seconds (more than we'll allow)
      const currentClock = await ctx.provider.context.banksClient.getClock();
      const newClock = new Clock(
        currentClock.slot + BigInt(200),
        currentClock.epochStartTimestamp,
        currentClock.epoch,
        currentClock.leaderScheduleEpoch,
        currentClock.unixTimestamp + BigInt(200)
      );
      ctx.provider.context.setClock(newClock);

      // Try to store a snapshot with max_age of 60 seconds
      // The last answer is now ~200 seconds old, so this should fail
      try {
        await ctx.feedConsumerProgram.methods
          .storeSnapshot(new BN(60)) // max_age = 60 seconds
          .accountsPartial({
            authority: ctx.authority.publicKey,
            feed: ctx.feedPDA,
            snapshot: snapshotPDA,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        assert.fail("Should have rejected stale feed");
      } catch (error: any) {
        // Check for the custom StaleFeed error
        assert.include(error.toString(), "StaleFeed");
        console.log("✓ Correctly rejected stale feed data");
      }
    });

    xit("Accepts feed data within max_age threshold", async () => {
      // Reset clock back to normal
      const currentClock = await ctx.provider.context.banksClient.getClock();
      const newClock = new Clock(
        currentClock.slot + BigInt(10),
        currentClock.epochStartTimestamp,
        currentClock.epoch,
        currentClock.leaderScheduleEpoch,
        currentClock.unixTimestamp + BigInt(10)
      );
      ctx.provider.context.setClock(newClock);

      // Publish a fresh answer
      const price = 52000_00000000; // $52,000
      const conf = 10_00000000;
      const expo = -8;

      await publishAnswerToFeed(ctx, price, conf, expo);

      // This should succeed since the answer is fresh
      await ctx.feedConsumerProgram.methods
        .storeSnapshot(new BN(120)) // max_age = 120 seconds
        .accountsPartial({
          authority: ctx.authority.publicKey,
          feed: ctx.feedPDA,
          snapshot: snapshotPDA,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      const snapshot = await ctx.feedConsumerProgram.account.snapshot.fetch(
        snapshotPDA
      );

      const valueBuffer = Buffer.from(snapshot.value);
      const storedPrice = valueBuffer.readBigInt64LE(0);

      assert.equal(storedPrice.toString(), BigInt(price).toString());
      console.log("✓ Accepted fresh feed data with max_age check");
    });
  });

  describe("Close Snapshot", () => {
    xit("Successfully closes a snapshot and returns rent", async () => {
      // Get authority balance before closing
      const authorityAccountBefore = await ctx.provider.context.banksClient.getAccount(
        ctx.authority.publicKey
      );
      const balanceBefore = authorityAccountBefore
        ? Number(authorityAccountBefore.lamports)
        : 0;

      // Close the snapshot
      await ctx.feedConsumerProgram.methods
        .closeSnapshot()
        .accountsPartial({
          authority: ctx.authority.publicKey,
          snapshot: snapshotPDA,
        })
        .rpc();

      // Verify the snapshot account is closed
      const snapshotAccount = await ctx.provider.context.banksClient.getAccount(
        snapshotPDA
      );
      assert.isNull(snapshotAccount, "Snapshot account should be closed");

      // Verify rent was returned (balance should have increased)
      const authorityAccountAfter = await ctx.provider.context.banksClient.getAccount(
        ctx.authority.publicKey
      );
      const balanceAfter = authorityAccountAfter
        ? Number(authorityAccountAfter.lamports)
        : 0;

      assert.isTrue(
        balanceAfter > balanceBefore,
        "Rent should be returned to authority"
      );

      console.log("✓ Snapshot closed successfully");
      console.log(`  Balance increase: ${balanceAfter - balanceBefore} lamports`);
    });

    xit("Fails to close a non-existent snapshot", async () => {
      // Try to close the already-closed snapshot
      try {
        await ctx.feedConsumerProgram.methods
          .closeSnapshot()
          .accountsPartial({
            authority: ctx.authority.publicKey,
            snapshot: snapshotPDA,
          })
          .rpc();

        assert.fail("Should have failed to close non-existent snapshot");
      } catch (error: any) {
        // Should get an account not found error
        assert.include(
          error.toString().toLowerCase(),
          "account"
        );
        console.log("✓ Correctly failed to close non-existent snapshot");
      }
    });
  });

  describe("Integration Test: Full Flow", () => {
    xit("Complete flow: create feed → publish answer → store snapshot → close", async () => {
      // Create a new feed for this integration test
      const integrationFeedName = "integration-test-feed";
      const feedParams = {
        jobId: Array.from(Buffer.from(integrationFeedName.padEnd(32, "\0"))),
        feedType: { public: {} },
        minSignaturesThreshold: 1,
        frequency: new anchor.BN(60),
        ipfsCid: "QmIntegrationTest",
        name: integrationFeedName,
      };

      const [integrationFeedPDA] = getFeedPda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        feedParams.name,
        0
      );

      // Create the feed
      await ctx.molphaProgram.methods
        .createFeed(feedParams, new anchor.BN(86400))
        .accountsPartial({
          feed: integrationFeedPDA,
          dataSource: ctx.dataSourcePDA,
          underlyingToken: ctx.molphaProgram.provider.publicKey, // Mock
          userTokenAccount: ctx.authority.publicKey, // Mock for now
          programTokenAccount: ctx.authority.publicKey, // Mock for now
          protocolConfig: PublicKey.findProgramAddressSync(
            [Buffer.from("config")],
            ctx.molphaProgram.programId
          )[0],
          authority: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      console.log("✓ Created integration test feed");

      // Publish answer
      const price = 65000_00000000; // $65,000
      const conf = 15_00000000; // $15 confidence
      const expo = -8;

      const clock = await ctx.provider.context.banksClient.getClock();
      const timestamp = Number(clock.unixTimestamp);

      const value = Buffer.alloc(32);
      value.writeBigInt64LE(BigInt(price), 0);
      value.writeBigUInt64LE(BigInt(conf), 8);
      value.writeInt32LE(expo, 16);

      const answer = {
        value: Array.from(value),
        timestamp: new BN(timestamp),
      };

      await ctx.molphaProgram.methods
        .publishAnswer(answer)
        .accountsPartial({
          feed: integrationFeedPDA,
          node: ctx.nodePDA,
          authority: ctx.authority.publicKey,
        })
        .remainingAccounts([
          {
            pubkey: ctx.nodeKeypair.publicKey,
            isSigner: false,
            isWritable: false,
          },
        ])
        .rpc();

      console.log("✓ Published answer to feed");

      // Store snapshot
      const [integrationSnapshotPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("snapshot"),
          ctx.authority.publicKey.toBuffer(),
          integrationFeedPDA.toBuffer(),
        ],
        ctx.feedConsumerProgram.programId
      );

      await ctx.feedConsumerProgram.methods
        .storeSnapshot(new BN(300)) // 5 minute max age
        .accountsPartial({
          authority: ctx.authority.publicKey,
          feed: integrationFeedPDA,
          snapshot: integrationSnapshotPDA,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      console.log("✓ Stored snapshot");

      // Verify snapshot
      const snapshot = await ctx.feedConsumerProgram.account.snapshot.fetch(
        integrationSnapshotPDA
      );

      const valueBuffer = Buffer.from(snapshot.value);
      const storedPrice = valueBuffer.readBigInt64LE(0);

      assert.equal(storedPrice.toString(), BigInt(price).toString());
      console.log(`✓ Verified snapshot data (Price: ${storedPrice})`);

      // Close snapshot
      await ctx.feedConsumerProgram.methods
        .closeSnapshot()
        .accountsPartial({
          authority: ctx.authority.publicKey,
          snapshot: integrationSnapshotPDA,
        })
        .rpc();

      console.log("✓ Closed snapshot");

      // Verify closed
      const closedAccount = await ctx.provider.context.banksClient.getAccount(
        integrationSnapshotPDA
      );
      assert.isNull(closedAccount);

      console.log("✅ Full integration test completed successfully");
    });
  });
});
