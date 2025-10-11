import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram, Transaction } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import nacl from "tweetnacl";
import { BankrunProvider } from "anchor-bankrun";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  getDataSourcePda,
  createTestDataSourceInfo,
  registerNodeWithDeposit,
} from "../setup";

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

describe("Publish Answer Instruction", () => {
  let ctx: TestContext;
  let publicFeedPDA: PublicKey;
  let personalFeedPDA: PublicKey;
  let dataSourcePDA: PublicKey;

  const publicFeedId = "public-feed-publish-test";
  const personalFeedId = "personal-feed-publish-test";

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);

    // Register nodes for signature verification
    for (let i = 0; i < 3; i++) {
      try {
        const evmPubkey = Buffer.alloc(33, i + 1);
        await registerNodeWithDeposit(ctx, ctx.nodes[i].publicKey, evmPubkey);
      } catch (e) {
        // Ignore error if node already registered
      }
    }

    const dataSourceInfo = {
      dataSourceType: { public: {} },
      source: "https://api.example.com/price",
      owner: ctx.authority.publicKey,
      name: "Test Price Feed",
    };

    [dataSourcePDA] = getDataSourcePda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      dataSourceInfo.name,
      0
    );

    // Create data source
    try {
      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: dataSourcePDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();
    } catch (e) {
      // Ignore if already exists
    }
    // Create public feed

    const publicFeedParams = {
      jobId: Array.from(Buffer.from(publicFeedId.padEnd(32, "\0"))),
      feedType: { public: {} },
      minSignaturesThreshold: 2,
      frequency: new anchor.BN(300),
      ipfsCid: "QmTestPublic",
      name: publicFeedId,
    };

    [publicFeedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(publicFeedParams.name),
        Buffer.from([0]), // FeedType::Public = 0
        Buffer.from([publicFeedParams.minSignaturesThreshold]),
        publicFeedParams.frequency.toBuffer("le", 8),
        Buffer.from(publicFeedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    try {
      await ctx.molphaProgram.methods
        .createFeed(publicFeedParams, new anchor.BN(86400))
        .accountsPartial({
          feed: publicFeedPDA,
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
    } catch (e) {
      // Ignore if already exists
    }

    // Create personal feed
    const personalFeedParams = {
      jobId: Array.from(Buffer.from(personalFeedId.padEnd(32, "\0"))),
      feedType: { personal: {} },
      minSignaturesThreshold: 2,
      frequency: new anchor.BN(300),
      ipfsCid: "QmTestPersonal",
      name: personalFeedId,
    };

    [personalFeedPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("feed"),
        ctx.authority.publicKey.toBuffer(),
        Buffer.from(personalFeedParams.name),
        Buffer.from([1]), // FeedType::Personal = 1
        Buffer.from([personalFeedParams.minSignaturesThreshold]),
        personalFeedParams.frequency.toBuffer("le", 8),
        Buffer.from(personalFeedParams.jobId),
      ],
      ctx.molphaProgram.programId
    );

    await ctx.molphaProgram.methods
      .createFeed(personalFeedParams, new anchor.BN(86400))
      .accountsPartial({
        feed: personalFeedPDA,
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
  });

  describe("Basic Publishing", () => {
    it("Successfully publishes answer to public feed with valid signatures", async () => {
      const answer = {
        value: Array.from(
          Buffer.from(
            "1000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ), // 32 bytes
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          100
        ),
      };

      // The message must be the answer value for signature verification
      const message = Buffer.from(answer.value);

      const signers = [ctx.nodes[0], ctx.nodes[1]]; // Use 2 signers to meet threshold

      // Add Ed25519 signature instructions
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
          feed: publicFeedPDA,
          instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
        })
        .remainingAccounts(nodePDAs)
        .preInstructions(preIxs)
        .rpc();

      // Verify the answer was published
      const feed = await ctx.molphaProgram.account.feed.fetch(publicFeedPDA);
      assert.deepEqual(feed.latestAnswer.value, answer.value);
      assert.ok(feed.latestAnswer.timestamp.eq(answer.timestamp));

      // Verify answer history was updated
      assert.equal(feed.answerHistory.length, 1);
      assert.deepEqual(feed.answerHistory[0].value, answer.value);
    });

    it("Successfully publishes answer to personal feed with subscription", async () => {
      const answer = {
        value: Array.from(
          Buffer.from(
            "2000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ), // 32 bytes
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          90
        ), // 5 seconds ago
      };

      // The message must be the answer value for signature verification
      const message = Buffer.from(answer.value);

      // Get initial feed balance
      const initialFeed = await ctx.molphaProgram.account.feed.fetch(
        personalFeedPDA
      );

      const signers = [ctx.nodes[0], ctx.nodes[1]];

      // Add Ed25519 signature instructions
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
          feed: personalFeedPDA,
          instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
        })
        .remainingAccounts(nodePDAs)
        .preInstructions(preIxs)
        .rpc();

      // Verify the answer was published
      const feed = await ctx.molphaProgram.account.feed.fetch(personalFeedPDA);
      assert.deepEqual(feed.latestAnswer.value, answer.value);
      assert.ok(feed.latestAnswer.timestamp.eq(answer.timestamp));
    });
  });

  describe("Signature Validation", () => {
    it("Fails with insufficient signatures", async () => {
      const answer = {
        value: Array.from(
          Buffer.from(
            "3000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          80
        ), // 3 seconds ago
      };

      // The message must be the answer value for signature verification
      const message = Buffer.from(answer.value);

      const signers = [ctx.nodes[0]];
      const preIxs = signers.map((signer) => {
        const signature = nacl.sign.detached(message, signer.secretKey);
        return anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message,
          signature,
        });
      });

      // Get Node PDAs for the signer  
      const nodePDAs = [ctx.nodes[0]].map((signer) => {
        const [nodePDA] = PublicKey.findProgramAddressSync(
          [Buffer.from("node"), signer.publicKey.toBuffer()],
          ctx.molphaProgram.programId
        );
        return { pubkey: nodePDA, isSigner: false, isWritable: false };
      });

      try {
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: personalFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .preInstructions(preIxs)
          .rpc();
        assert.fail("Should have failed with insufficient signatures");
      } catch (error: any) {
        const feed = await ctx.molphaProgram.account.feed.fetch(
          personalFeedPDA
        );
        console.log(
          "feed.latestAnswer.timestamp:",
          feed.latestAnswer.timestamp.toNumber()
        );
        console.log("answer.timestamp:", answer.timestamp.toNumber());
        console.log("myerror:", JSON.stringify(error));
        assert.ok(
          error.message.includes("NotEnoughSignatures") ||
            error.message.includes("custom program error")
        );
      }
    });

    it("Fails with signatures from non-registered nodes", async () => {
      const answer = {
        value: Array.from(
          Buffer.from(
            "4000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          70
        ), // 2 seconds ago
      };

      // The message must be the answer value for signature verification
      const message = Buffer.from(answer.value);

      const transaction = new Transaction();

      // Use unregistered nodes
      const unregisteredNode1 = anchor.web3.Keypair.generate();
      const unregisteredNode2 = anchor.web3.Keypair.generate();

      const signers = [unregisteredNode1, unregisteredNode2];

      // Add Ed25519 signature instructions
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

      try {
        // Build + send via Anchor (ensures correct accounts/ordering/provider)
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: personalFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .preInstructions(preIxs)
          .rpc();

        assert.fail("Should have failed with unregistered node signatures");
      } catch (error: any) {
        assert.ok(
          error.message.includes("NotEnoughSignatures") ||
            error.message.includes("custom program error")
        );
      }
    });

    it("Fails with wrong message in signatures", async () => {
      const answer = {
        value: Array.from(
          Buffer.from(
            "5000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: await safePastOnchainTimestamp(
          ctx.molphaProgram.provider,
          60
        ), // 1 second ago
      };

      // Sign a different message (not the answer value)
      const wrongMessage = Buffer.from("wrong-message-not-answer-value");

      const signers = [ctx.nodes[0], ctx.nodes[1]];

      // Add Ed25519 signature instructions
      const preIxs = signers.map((signer) => {
        const signature = nacl.sign.detached(wrongMessage, signer.secretKey);
        return anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message: wrongMessage,
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

      try {
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: publicFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .preInstructions(preIxs)
          .rpc();
        assert.fail("Should have failed with wrong message signature");
      } catch (error: any) {
        assert.ok(
          error.message.includes("NotEnoughSignatures") ||
            error.message.includes("custom program error")
        );
      }
    });
  });

  describe("Timestamp Validation", () => {
    it("Fails with past timestamp", async () => {
      const message = Buffer.from("test-message-past");

      // Get current latest answer timestamp
      const currentFeed = await ctx.molphaProgram.account.feed.fetch(
        publicFeedPDA
      );
      const pastTimestamp = currentFeed.latestAnswer.timestamp.sub(
        new anchor.BN(1)
      );

      const answer = {
        value: Array.from(
          Buffer.from(
            "6000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: pastTimestamp,
      };

      const transaction = new Transaction();
      const signers = [ctx.nodes[0], ctx.nodes[1]];

      for (const signer of signers) {
        const signature = nacl.sign.detached(message, signer.secretKey);
        const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message,
          signature,
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
            feed: publicFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .instruction()
      );

      try {
        await ctx.provider.sendAndConfirm(transaction, [ctx.authority.payer]);
        assert.fail("Should have failed with past timestamp");
      } catch (error: any) {
        assert.ok(
          error.message.includes("PastTimestamp") ||
            error.message.includes("custom program error")
        );
      }
    });

    it("Fails with future timestamp", async () => {
      const message = Buffer.from("test-message-future");
      const futureTimestamp = new anchor.BN(
        Math.floor(Date.now() / 1000) + 3600
      ); // 1 hour in future

      const answer = {
        value: Array.from(
          Buffer.from(
            "7000000000000000000000000000000000000000000000000000000000000000",
            "hex"
          )
        ),
        timestamp: futureTimestamp,
      };

      const transaction = new Transaction();
      const signers = [ctx.nodes[0], ctx.nodes[1]];

      for (const signer of signers) {
        const signature = nacl.sign.detached(message, signer.secretKey);
        const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
          publicKey: signer.publicKey.toBytes(),
          message,
          signature,
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
            feed: publicFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .instruction()
      );

      try {
        await ctx.provider.sendAndConfirm(transaction, [ctx.authority.payer]);
        assert.fail("Should have failed with future timestamp");
      } catch (error: any) {
        assert.ok(
          error.message.includes("FutureTimestamp") ||
            error.message.includes("custom program error")
        );
      }
    });
  });

  describe("Answer History Management", () => {
    it("Maintains answer history with ring buffer", async () => {
      // Create a new feed for history testing
      const historyFeedId = "history-test-feed";
      const historyFeedName = historyFeedId; // Use the same value for consistency
      const historyFeedParams = {
        jobId: Array.from(Buffer.from(historyFeedId.padEnd(32, "\0"))),
        feedType: { public: {} },
        minSignaturesThreshold: 2,
        frequency: new anchor.BN(300),
        ipfsCid: "QmHistoryTest",
        name: historyFeedName,
      };

      const [historyFeedPDA] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("feed"),
          ctx.authority.publicKey.toBuffer(),
          Buffer.from(historyFeedParams.name),
          Buffer.from([0]), // FeedType::Public.to_seed() = [0]
          Buffer.from([historyFeedParams.minSignaturesThreshold]), // u8.to_le_bytes() = [value]
          historyFeedParams.frequency.toBuffer("le", 8),
          Buffer.from(historyFeedParams.jobId),
        ],
        ctx.molphaProgram.programId
      );

      const dataSourceInfo = createTestDataSourceInfo(0, "History Test Feed");

      const [historyDataSourcePDA] = getDataSourcePda(
        ctx.molphaProgram.programId,
        ctx.authority.publicKey,
        dataSourceInfo.name,
        0
      );

      await ctx.molphaProgram.methods
        .createDataSource(dataSourceInfo as any)
        .accountsPartial({
          dataSource: historyDataSourcePDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      await ctx.molphaProgram.methods
        .createFeed(historyFeedParams, new anchor.BN(86400))
        .accountsPartial({
          feed: historyFeedPDA,
          dataSource: historyDataSourcePDA,
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

      // Publish multiple answers to test history
      const numAnswers = 5;
      for (let i = 0; i < numAnswers; i++) {
        const answer = {
          value: Array.from(
            Buffer.from(`${i.toString().padStart(64, "0")}`, "hex")
          ),
          timestamp: new anchor.BN(Math.floor(Date.now() / 1000) - 100 + i), // Start from 100 seconds ago
        };

        // The message must be the answer value for signature verification
        const message = Buffer.from(answer.value);

        // Create signature instructions like the working tests
        const preIxs: anchor.web3.TransactionInstruction[] = [];
        const signers = [ctx.nodes[0], ctx.nodes[1]];

        signers.forEach((signer) => {
          const signature = nacl.sign.detached(message, signer.secretKey);
          const ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
            publicKey: signer.publicKey.toBytes(),
            message,
            signature,
          });
          preIxs.push(ix);
        });

        // Get Node PDAs for the signers
        const nodePDAs = signers.map((signer) => {
          const [nodePDA] = PublicKey.findProgramAddressSync(
            [Buffer.from("node"), signer.publicKey.toBuffer()],
            ctx.molphaProgram.programId
          );
          return { pubkey: nodePDA, isSigner: false, isWritable: false };
        });

        // Use the same pattern as working tests
        await ctx.molphaProgram.methods
          .publishAnswer(answer)
          .accountsPartial({
            feed: historyFeedPDA,
            instructions: anchor.web3.SYSVAR_INSTRUCTIONS_PUBKEY,
          })
          .remainingAccounts(nodePDAs)
          .preInstructions(preIxs)
          .rpc();

        // Small delay to ensure timestamp progression
        await new Promise((resolve) => setTimeout(resolve, 100));
      }

      // Verify history was maintained
      const feed = await ctx.molphaProgram.account.feed.fetch(historyFeedPDA);
      assert.equal(feed.answerHistory.length, numAnswers);

      // Verify latest answer is correct
      const expectedLatestValue = Array.from(
        Buffer.from(`${(numAnswers - 1).toString().padStart(64, "0")}`, "hex")
      );
      assert.deepEqual(feed.latestAnswer.value, expectedLatestValue);

      // Verify history contains all answers in order
      for (let i = 0; i < numAnswers; i++) {
        const expectedValue = Array.from(
          Buffer.from(`${i.toString().padStart(64, "0")}`, "hex")
        );
        assert.deepEqual(feed.answerHistory[i].value, expectedValue);
      }
    });
  });
});
