import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram, Transaction } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import nacl from "tweetnacl";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  createTestDataSourceInfo,
  getDataSourcePda,
  registerNodeWithDeposit,
} from "../setup";
import { BankrunProvider } from "anchor-bankrun";

async function safePastOnchainTimestamp(
  provider: BankrunProvider | any, // works with BankrunProvider too
  secondsInPast = 1
): Promise<anchor.BN> {
  const client = provider.connection.banksClient.inner;
  const clock = await client.getClock();
  const timestamp = new anchor.BN(
    Math.max(0, Number(clock.unixTimestamp) - secondsInPast)
  );
  return timestamp;
}

describe("Integration Tests: Complete Oracle Flow", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  describe("Complete Flow: Add Nodes → Create Data Source → Create Feed → Publish Data", () => {
    it("Executes complete public feed workflow successfully", async () => {
      // Step 1: Register nodes
      console.log("Step 1: Registering nodes...");
      const nodesToAdd = [ctx.nodes[0], ctx.nodes[1], ctx.nodes[2]]; // Add 3 nodes

      for (const node of nodesToAdd) {
        const evmPubkey = Buffer.alloc(33, 1);

        try {
          await registerNodeWithDeposit(ctx, node.publicKey, evmPubkey);
        } catch (e) {
          // Ignore if already registered
        }
      }

      // Verify nodes were registered
      for (const node of nodesToAdd) {
        const [nodePDA] = PublicKey.findProgramAddressSync(
          [Buffer.from("node"), node.publicKey.toBuffer()],
          ctx.molphaProgram.programId
        );
        const nodeAccount = await ctx.molphaProgram.account.node.fetch(nodePDA);
        assert.ok(nodeAccount.nodePubkey.equals(node.publicKey));
      }

      const testData = {
        dataSourceType: 0, // Public
        source: "https://api.kraken.com/0/public/Ticker?pair=XBTUSD",
        name: "Kraken Bitcoin Price",
      };

      const dataSourceInfo = createTestDataSourceInfo(
        testData.dataSourceType,
        testData.name
      );

      const [dataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        testData.name,
        testData.dataSourceType
      );

      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: dataSourcePDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      // Verify data source was created
      const dataSourceAccount =
        await ctx.molphaProgram.account.dataSource.fetch(dataSourcePDA);
      assert.deepEqual(dataSourceAccount.authority, ctx.authority.publicKey);
      assert.deepEqual(dataSourceAccount.dataSourceType, { public: {} });

      // Step 3: Create feed using the data source
      console.log("Step 3: Creating feed...");
      const feedId = "integration-kraken-feed";
      const feedParams = {
        name: feedId,
        jobId: Array.from(Buffer.from(feedId.padEnd(32, "\0"))),
        feedType: { public: {} },
        minSignaturesThreshold: 2, // Require 2 out of 3 nodes
        frequency: new anchor.BN(600), // 10 minutes
        ipfsCid: "QmIntegrationTest123",
      };

      const [feedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(feedParams.name),
          Buffer.from([0]), // FeedType::Public = 0
          Buffer.from([feedParams.minSignaturesThreshold]),
          feedParams.frequency.toBuffer("le", 8),
          Buffer.from(feedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

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

      // Verify feed was created
      console.log("Verifying feed was created...");
      console.log("feedPDA:", feedPDA.toBase58());
      const feed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      assert.ok(feed.authority.equals(ctx.authority.publicKey));
      assert.deepEqual(feed.feedType, { public: {} });
      assert.deepEqual(feed.dataSource, dataSourcePDA);
      assert.equal(feed.minSignaturesThreshold, 2);
      assert.equal(feed.frequency.toNumber(), 600);
      assert.equal(feed.ipfsCid, "QmIntegrationTest123");

      // Step 4: Publish data to the feed
      console.log("Step 4: Publishing data to feed...");
      const answer = {
        value: Array.from(
          Buffer.from(
            "5000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ), // 32 bytes representing price data
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          10
        ), // 10 seconds ago
      };

      // Create message for signature verification (must match answer.value)
      const message = Buffer.from(answer.value);

      // Create transaction with Ed25519 signatures from 2 nodes
      const signers = [nodesToAdd[0], nodesToAdd[1]]; // Use 2 signers to meet threshold

      // Add publish answer instruction
      // Create Ed25519 pre-instructions from 2 nodes
      const preIxs = signers.map((signer) => {
        const signature = nacl.sign.detached(message, signer.secretKey);
        return anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message,
          signature,
        });
      });

      // Get Node PDAs for the signers
      const nodePDAs = signers.map((signer) => {
        const [nodePDA] = PublicKey.findProgramAddressSync(
          [Buffer.from("node"), signer.publicKey.toBuffer()],
          ctx.molphaProgram.programId
        );
        return { pubkey: nodePDA, isSigner: false, isWritable: false };
      });

      // Build + send via Anchor (ensures correct accounts/ordering/provider)
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
      assert.deepEqual(updatedFeed.latestAnswer.value, answer.value);
      assert.ok(updatedFeed.latestAnswer.timestamp.eq(answer.timestamp));

      // Verify answer history was updated
      assert.equal(updatedFeed.answerHistory.length, 1);
      assert.deepEqual(updatedFeed.answerHistory[0].value, answer.value);

      console.log("✅ Complete public feed workflow executed successfully!");
    });

    it("Executes complete personal feed workflow with subscription", async () => {
      // Step 1: Register nodes (reuse from previous test)
      console.log("Step 1: Registering nodes...");
      const nodesToAdd = [ctx.nodes[0], ctx.nodes[1], ctx.nodes[2]];

      for (const node of nodesToAdd) {
        try {
          const evmPubkey = Buffer.alloc(33, 1);
          await registerNodeWithDeposit(ctx, node.publicKey, evmPubkey);
        } catch (e) {
          // Ignore if already registered
        }
      }

      const testData = {
        dataSourceType: 1, // Private
        source: "https://private-api.example.com/btc-price",
        name: "Private API Integration Test",
      };

      const dataSourceInfo = createTestDataSourceInfo(
        testData.dataSourceType,
        testData.name
      );

      const [dataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        dataSourceInfo.name,
        1
      );

      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: dataSourcePDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      // Step 3: Create personal feed
      console.log("Step 3: Creating personal feed...");
      const feedId = "integration-private-feed";
      const feedParams = {
        name: feedId,
        jobId: Array.from(Buffer.from(feedId.padEnd(32, "\0"))),
        feedType: { personal: {} },
        minSignaturesThreshold: 1, // Require only 1 signature for personal feed
        frequency: new anchor.BN(900), // 15 minutes
        ipfsCid: "QmPrivateIntegrationTest",
      };

      const [feedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(feedParams.name),
          Buffer.from([1]), // FeedType::Personal = 1
          Buffer.from([feedParams.minSignaturesThreshold]),
          feedParams.frequency.toBuffer("le", 8),
          Buffer.from(feedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

      const subscriptionDurationSeconds = new anchor.BN(86400); // 1 day
      const priorityFeeBudget = new anchor.BN(2000); // 2000 lamports

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

      // Step 4: Publish data to the personal feed
      console.log("Step 5: Publishing data to personal feed...");
      const answer = {
        value: Array.from(
          Buffer.from(
            "7500000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ), // 32 bytes representing different price data
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          5
        ), // 5 seconds ago
      };

      const message = Buffer.from(answer.value);

      // Create signature instructions like the working tests
      const preIxs: anchor.web3.TransactionInstruction[] = [];
      const signer = nodesToAdd[0]; // Use 1 signer to meet threshold

      // Add Ed25519 signature instruction
      const signature_bytes = nacl.sign.detached(message, signer.secretKey);
      const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
        publicKey: signer.publicKey.toBytes(),
        message,
        signature: signature_bytes,
      });
      preIxs.push(ix);

      // Get Node PDA for the signer
      const nodePDA = PublicKey.findProgramAddressSync(
        [Buffer.from("node"), signer.publicKey.toBuffer()],
        ctx.molphaProgram.programId
      )[0];

      // Use the same pattern as working tests
      await ctx.molphaProgram.methods
        .publishAnswer(answer)
        .accountsPartial({
          feed: feedPDA,
          instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
        })
        .remainingAccounts([
          { pubkey: nodePDA, isSigner: false, isWritable: false },
        ])
        .preInstructions(preIxs)
        .rpc();

      // Verify the answer was published
      const updatedFeed = await ctx.molphaProgram.account.feed.fetch(feedPDA);
      assert.deepEqual(updatedFeed.latestAnswer.value, answer.value);
      assert.ok(updatedFeed.latestAnswer.timestamp.eq(answer.timestamp));

      console.log("✅ Complete personal feed workflow executed successfully!");
    });

    it("Fails to publish data with insufficient signatures", async () => {
      // Create a feed with high signature threshold
      const feedId = "high-threshold-feed";

      const dataSourceInfo = createTestDataSourceInfo(0, "High Threshold Test");

      const [dataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        dataSourceInfo.name,
        0
      );

      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: dataSourcePDA,
          authority: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      const feedParams = {
        name: feedId,
        jobId: Array.from(Buffer.from(feedId.padEnd(32, "\0"))),
        feedType: { public: {} },
        minSignaturesThreshold: 3, // Require 3 signatures
        frequency: new anchor.BN(300),
        ipfsCid: "QmHighThreshold",
      };

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

      const subscriptionDurationSeconds = new anchor.BN(86400);

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

      // Try to publish with only 2 signatures (below threshold of 3)
      const answer = {
        value: Array.from(
          Buffer.from(
            "1000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          10
        ),
      };

      const message = Buffer.from(answer.value);
      const transaction = new Transaction();

      // Add only 2 signatures (below threshold)
      const signers = [ctx.nodes[0], ctx.nodes[1]];
      for (const signer of signers) {
        const signature_bytes = nacl.sign.detached(message, signer.secretKey);
        const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message,
          signature: signature_bytes,
        });
        transaction.add(ix);
      }

      // Get Node PDAs for the signers
      const nodePDAs = signers.map((signer) => {
        const [nodePDA] = PublicKey.findProgramAddressSync(
          [Buffer.from("node"), signer.publicKey.toBuffer()],
          ctx.molphaProgram.programId
        );
        return { pubkey: nodePDA, isSigner: false, isWritable: false };
      });

      transaction.add(
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: feedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .instruction()
      );

      // This should fail due to insufficient signatures
      try {
        await ctx.provider.sendAndConfirm(transaction, [ctx.authority.payer]);
        assert.fail("Should have failed due to insufficient signatures");
      } catch (error: any) {
        assert.ok(
          error.message.includes("NotEnoughSignatures") ||
            error.message.includes("custom program error"),
          "Should fail with signature threshold error"
        );
      }

      console.log(
        "✅ Correctly failed to publish with insufficient signatures!"
      );
    });

    it("Demonstrates multi-feed data source sharing", async () => {
      const sharedDataSource = {
        dataSourceType: 0,
        source:
          "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
        name: "Shared CoinGecko Data Source",
      };

      const dataSourceInfo = createTestDataSourceInfo(
        sharedDataSource.dataSourceType,
        sharedDataSource.name
      );

      const [dataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        sharedDataSource.name,
        sharedDataSource.dataSourceType
      );

      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: dataSourcePDA,
          authority: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      // Create multiple feeds using the same data source
      const feedConfigs = [
        {
          id: "shared-feed-public",
          type: { public: {} },
          threshold: 2,
          frequency: 300,
        },
        {
          id: "shared-feed-personal",
          type: { personal: {} },
          threshold: 1,
          frequency: 600,
        },
      ];

      const createdFeeds = [];

      for (const config of feedConfigs) {
        const feedParams = {
          name: config.id,
          jobId: Array.from(Buffer.from(config.id.padEnd(32, "\0"))),
          feedType: config.type,
          minSignaturesThreshold: config.threshold,
          frequency: new anchor.BN(config.frequency),
          ipfsCid: `QmShared${config.id}`,
        };

        const feedTypeValue = config.type.public ? 0 : 1;
        const [feedPDA] = PublicKey.findProgramAddressSync(
          [
            Buffer.from("feed"),
            ctx.authority.publicKey.toBuffer(),
            Buffer.from(feedParams.name),
            Buffer.from([feedTypeValue]),
            Buffer.from([feedParams.minSignaturesThreshold]),
            feedParams.frequency.toBuffer("le", 8),
            Buffer.from(feedParams.jobId),
          ],
          ctx.molphaProgram.programId
        );

        const subscriptionDurationSeconds = new anchor.BN(86400);
        const priorityFeeBudget = new anchor.BN(1000);

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

        createdFeeds.push({ pda: feedPDA, config });
      }

      // Publish data to both feeds
      for (const { pda, config } of createdFeeds) {
        const answer = {
          value: Array.from(
            Buffer.from(
              "8000000000000000000000000000000000000000000000000000000000000000",
              "hex"
            )
          ),
          timestamp: await safePastOnchainTimestamp(
            ctx.molphaProgram.provider,
            15
          ),
        };

        const message = Buffer.from(answer.value);

        // Create signature instructions like the working tests
        const preIxs: anchor.web3.TransactionInstruction[] = [];
        const numSignatures = config.threshold;
        for (let i = 0; i < numSignatures; i++) {
          const signer = ctx.nodes[i];
          const signature_bytes = nacl.sign.detached(message, signer.secretKey);
          const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
            publicKey: signer.publicKey.toBytes(),
            message,
            signature: signature_bytes,
          });
          preIxs.push(ix);
        }

        // Get Node PDAs for the signers
        const nodePDAs = [];
        for (let i = 0; i < numSignatures; i++) {
          const [nodePDA] = PublicKey.findProgramAddressSync(
            [Buffer.from("node"), ctx.nodes[i].publicKey.toBuffer()],
            ctx.molphaProgram.programId
          );
          nodePDAs.push({
            pubkey: nodePDA,
            isSigner: false,
            isWritable: false,
          });
        }

        // Use the same pattern as working tests
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: pda,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .preInstructions(preIxs)
          .rpc();

        // Verify data was published
        const feed = await ctx.molphaProgram.account.feed.fetch(pda);
        assert.deepEqual(feed.latestAnswer.value, answer.value);
        assert.deepEqual(feed.dataSource, dataSourcePDA);
      }

      console.log(
        "✅ Multi-feed data source sharing demonstrated successfully!"
      );
    });
  });
});
