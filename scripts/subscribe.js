#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { SystemProgram, PublicKey } = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");

async function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Feed address is required");
    console.log("Usage: node subscribe.js <FEED_ADDRESS> [DAYS]");
    console.log("  FEED_ADDRESS: The public key of the feed to extend subscription");
    console.log("  DAYS: Number of days to extend (default: 30)");
    process.exit(1);
  }

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Molpha;

  // Parse command line arguments
  let feedAddress;
  try {
    feedAddress = new PublicKey(process.argv[2]);
  } catch (e) {
    console.error("❌ Error: Invalid feed address");
    process.exit(1);
  }

  const days = parseInt(process.argv[3]) || 30;
  const additionalDurationSeconds = days * 86400; // days to seconds

  // Derive protocol config PDA
  const [protocolConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  // Get protocol config to find underlying token
  const protocolConfig = await program.account.protocolConfig.fetch(protocolConfigPDA);
  const underlyingToken = protocolConfig.underlyingToken;

  // Fetch feed to verify it exists and get info
  let feed;
  try {
    feed = await program.account.feed.fetch(feedAddress);
  } catch (e) {
    console.error("❌ Error: Feed not found at address:", feedAddress.toString());
    process.exit(1);
  }

  // Verify authority
  if (!feed.authority.equals(provider.wallet.publicKey)) {
    console.error("❌ Error: You are not the authority for this feed");
    console.log("Feed authority:", feed.authority.toString());
    console.log("Your wallet:", provider.wallet.publicKey.toString());
    process.exit(1);
  }

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
    console.log("🔄 Extending subscription...");
    console.log("Feed:", feedAddress.toString());
    console.log("Feed name:", feed.name);
    console.log("Extension:", days, "days");
    console.log("Current subscription due time:", new Date(feed.subscriptionDueTime * 1000).toISOString());

    const txSignature = await program.methods
      .extendSubscription(
        new anchor.BN(additionalDurationSeconds)
      )
      .accounts({
        feed: feedAddress,
        authority: provider.wallet.publicKey,
        userTokenAccount,
        programTokenAccount,
        protocolConfig: protocolConfigPDA,
        underlyingToken,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    // Fetch updated feed
    const updatedFeed = await program.account.feed.fetch(feedAddress);

    console.log("✅ Subscription extended successfully!");
    console.log("Feed:", feedAddress.toString());
    console.log("Extended by:", days, "days");
    console.log("New due time:", new Date(updatedFeed.subscriptionDueTime * 1000).toISOString());
    console.log("Transaction signature:", txSignature);
  } catch (e) {
    console.error("❌ Error extending subscription:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs.join("\n"));
    }
    process.exit(1);
  }
}

main().catch(console.error);
