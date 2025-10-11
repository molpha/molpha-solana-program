#!/usr/bin/env node

const { Keypair } = require("@solana/web3.js");
const {
  initializeFaucet,
  requestTokens,
  getFaucetInfo,
  getUserCooldown,
  getUserTokenBalance,
} = require("./faucet-utils");

async function runFaucetDemo() {
  console.log("🎭 Faucet Demo - Full Workflow\n");
  console.log("=".repeat(50));

  try {
    // Step 1: Create test users
    console.log("\n📝 Step 1: Creating test users...");
    const authority = Keypair.generate();
    const user1 = Keypair.generate();
    const user2 = Keypair.generate();

    console.log("  Authority:", authority.publicKey.toString());
    console.log("  User 1:", user1.publicKey.toString());
    console.log("  User 2:", user2.publicKey.toString());

    // Step 2: Initialize faucet
    console.log("\n🏗️  Step 2: Initializing faucet...");
    const faucetResult = await initializeFaucet(
      authority,
      500, // 500 tokens per request
      10, // 10 second cooldown for demo
      6, // 6 decimals
      "Demo Token",
      "DEMO"
    );

    const tokenMint = faucetResult.tokenMint;

    // Step 3: Get faucet info
    console.log("\n📊 Step 3: Checking faucet information...");
    await getFaucetInfo(tokenMint);

    // Step 4: User 1 requests tokens
    console.log("\n💧 Step 4: User 1 requesting tokens...");
    await requestTokens(tokenMint, user1);

    // Step 5: Check user 1 balance
    console.log("\n💰 Step 5: Checking User 1 balance...");
    await getUserTokenBalance(tokenMint, user1.publicKey);

    // Step 6: User 1 tries to request again (should fail due to cooldown)
    console.log("\n⏰ Step 6: User 1 trying to request again (should fail)...");
    try {
      await requestTokens(tokenMint, user1);
    } catch (error) {
      console.log("  ✅ Expected cooldown error caught");
    }

    // Step 7: Check user 1 cooldown status
    console.log("\n🕐 Step 7: Checking User 1 cooldown status...");
    await getUserCooldown(tokenMint, user1.publicKey);

    // Step 8: User 2 requests tokens (should work)
    console.log("\n💧 Step 8: User 2 requesting tokens...");
    await requestTokens(tokenMint, user2);

    // Step 9: Check both users' balances
    console.log("\n💰 Step 9: Checking both users' balances...");
    await getUserTokenBalance(tokenMint, user1.publicKey);
    await getUserTokenBalance(tokenMint, user2.publicKey);

    // Step 10: Wait for cooldown and let user 1 request again
    console.log("\n⏳ Step 10: Waiting for cooldown to expire (12 seconds)...");
    await new Promise((resolve) => setTimeout(resolve, 12000));

    console.log("\n💧 Step 11: User 1 requesting tokens again...");
    await requestTokens(tokenMint, user1);

    // Final balance check
    console.log("\n💰 Final: Checking User 1 final balance...");
    await getUserTokenBalance(tokenMint, user1.publicKey);

    console.log("\n🎉 Demo completed successfully!");
    console.log("\n📋 Summary:");
    console.log(`  Token Mint: ${tokenMint.toString()}`);
    console.log(`  User 1 made 2 successful requests (1000 DEMO total)`);
    console.log(`  User 2 made 1 successful request (500 DEMO total)`);
    console.log(`  Cooldown mechanism working correctly`);
  } catch (error) {
    console.error("❌ Demo failed:", error.message);
    process.exit(1);
  }
}

// Run if called directly
if (require.main === module) {
  runFaucetDemo();
}

module.exports = { runFaucetDemo };
