#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { SystemProgram, PublicKey } = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
const { getFeedPda, getDataSourcePda } = require("./pda-helpers");
const crypto = require("crypto");

async function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Feed name is required");
    console.log(
      "Usage: node create-feed.js <FEED_NAME> <DATA_SOURCE_NAME> [TYPE] [MIN_SIGS] [FREQUENCY] [IPFS_CID] [SUBSCRIPTION_DAYS]"
    );
    console.log('  FEED_NAME: name of the feed (will be hashed for PDA)');
    console.log('  DATA_SOURCE_NAME: name of the data source to use');
    console.log('  TYPE: "personal" or "public" (default: public)');
    console.log("  MIN_SIGS: minimum signatures threshold (default: 1)");
    console.log("  FREQUENCY: update frequency in seconds (default: 60)");
    console.log("  IPFS_CID: IPFS content identifier (default: QmTestCid)");
    console.log("  SUBSCRIPTION_DAYS: subscription duration in days (default: 30)");
    process.exit(1);
  }

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Molpha;

  // Parse command line arguments
  const feedName = process.argv[2];
  const dataSourceName = process.argv[3];
  if (!dataSourceName) {
    console.error("❌ Error: Data source name is required");
    process.exit(1);
  }
  
  const feedTypeStr = process.argv[4] || "public";
  const feedType = feedTypeStr === "personal" ? { personal: {} } : { public: {} };
  const feedTypeNum = feedTypeStr === "personal" ? 1 : 0;
  const minSigs = parseInt(process.argv[5]) || 1;
  const frequency = parseInt(process.argv[6]) || 60;
  const ipfsCid = process.argv[7] || "QmTestCid";
  const subscriptionDays = parseInt(process.argv[8]) || 30;
  const subscriptionDurationSeconds = subscriptionDays * 86400; // days to seconds

  // Generate a random job ID (32 bytes)
  const jobId = Array.from(crypto.randomBytes(32));

  // Derive PDAs
  const [protocolConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  // Get protocol config to find underlying token
  const protocolConfig = await program.account.protocolConfig.fetch(protocolConfigPDA);
  const underlyingToken = protocolConfig.underlyingToken;

  // Derive data source PDA - assume public data source for now
  const [dataSourcePDA] = getDataSourcePda(
    program.programId,
    provider.wallet.publicKey,
    dataSourceName,
    0 // public data source
  );

  // Verify data source exists
  try {
    await program.account.dataSource.fetch(dataSourcePDA);
  } catch (e) {
    console.error("❌ Error: Data source not found:", dataSourcePDA.toString());
    console.log("💡 Create the data source first or check the name");
    process.exit(1);
  }

  // Derive feed PDA with all required seeds
  const [feedPDA] = getFeedPda(
    program.programId,
    provider.wallet.publicKey,
    feedName,
    feedTypeNum,
  );

  // Get token accounts
  const userTokenAccount = getAssociatedTokenAddressSync(
    underlyingToken,
    provider.wallet.publicKey
  );

  const programTokenAccount = getAssociatedTokenAddressSync(
    underlyingToken,
    protocolConfigPDA,
    true // allowOwnerOffCurve
  );

  try {
    console.log("🔄 Creating feed...");
    console.log("Feed name:", feedName);
    console.log("Feed PDA:", feedPDA.toString());
    console.log("Data source:", dataSourcePDA.toString());
    console.log("Subscription duration:", subscriptionDays, "days");

    const txSignature = await program.methods
      .createFeed(
        {
          name: feedName,
          jobId,
          feedType,
          minSignaturesThreshold: minSigs,
          frequency: new anchor.BN(frequency),
          ipfsCid,
        },
        new anchor.BN(subscriptionDurationSeconds)
      )
      .accounts({
        feed: feedPDA,
        dataSource: dataSourcePDA,
        authority: provider.wallet.publicKey,
        protocolConfig: protocolConfigPDA,
        userTokenAccount,
        programTokenAccount,
        underlyingToken,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    console.log("✅ Feed created successfully!");
    console.log("Feed name:", feedName);
    console.log("Feed type:", feedTypeStr);
    console.log("Feed PDA:", feedPDA.toString());
    console.log("Min signatures:", minSigs);
    console.log("Frequency:", frequency, "seconds");
    console.log("IPFS CID:", ipfsCid);
    console.log("Subscription days:", subscriptionDays);
    console.log("Transaction signature:", txSignature);
  } catch (e) {
    console.error("❌ Error creating feed:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs.join("\n"));
    }
    process.exit(1);
  }
}

main().catch(console.error);
