#!/usr/bin/env node

const { Keypair } = require("@solana/web3.js");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs");
const { initializeFaucet, saveTokenMintKeypair } = require("./faucet-utils");

// // Setup environment variables if not set
// function setupEnvironment() {
//   if (!process.env.ANCHOR_PROVIDER_URL) {
//     process.env.ANCHOR_PROVIDER_URL = "http://127.0.0.1:8899";
//   }

//   if (!process.env.ANCHOR_WALLET) {
//     const defaultWallet = require("os").homedir() + "/.config/solana/id.json";
//     if (fs.existsSync(defaultWallet)) {
//       process.env.ANCHOR_WALLET = defaultWallet;
//     }
//   }
// }

// async function setupProgram() {
//   setupEnvironment();

//   const provider = anchor.AnchorProvider.env();
//   anchor.setProvider(provider);
//   const program = anchor.workspace.Molpha;

//   return { program, provider, nodeRegistryPDA, protocolConfigPDA };
// }

async function setupTestFaucet() {
  console.log("🚀 Setting up test faucet...\n");

  try {
    // Load authority keypair (assuming it's the default Anchor wallet)
    const authority = anchor.AnchorProvider.env().wallet.payer;
    console.log(`Authority: ${authority.publicKey.toString()}`);

    // Create faucet with default test parameters
    const result = await initializeFaucet(
      authority,
      1000, // 1000 tokens per request
      60, // 1 minute cooldown for testing
      6, // 6 decimals
      "Test USDC", // Token name
      "TUSDC" // Token symbol
    );

    // Save keypairs for later use
    const authorityPath = "./test-authority-keypair.json";
    const mintPath = "./test-mint-keypair.json";

    fs.writeFileSync(
      authorityPath,
      JSON.stringify(Array.from(authority.secretKey))
    );
    saveTokenMintKeypair(result.tokenMintKeypair, mintPath);

    console.log("\n🎉 Test faucet setup complete!");
    console.log("\n📋 Configuration:");
    console.log(`Authority: ${authority.publicKey.toString()}`);
    console.log(`Token Mint: ${result.tokenMint.toString()}`);
    console.log(`Faucet Config: ${result.faucetConfigPDA.toString()}`);
    console.log(`Amount per request: 1000 TUSDC`);
    console.log(`Cooldown: 1 minute`);

    console.log("\n💾 Files saved:");
    console.log(`Authority keypair: ${authorityPath}`);
    console.log(`Token mint keypair: ${mintPath}`);

    console.log("\n🔧 Usage examples:");
    console.log(`# Request tokens:`);
    console.log(
      `node scripts/faucet-cli.js request -m ${result.tokenMint.toString()} -u ./your-user-keypair.json`
    );
    console.log(`# Check faucet info:`);
    console.log(
      `node scripts/faucet-cli.js info -m ${result.tokenMint.toString()}`
    );
    console.log(`# Check your balance:`);
    console.log(
      `node scripts/faucet-cli.js balance -m ${result.tokenMint.toString()} -u YOUR_PUBLIC_KEY`
    );
  } catch (error) {
    console.error("❌ Failed to setup test faucet:", error.message);
    process.exit(1);
  }
}

// Run if called directly
if (require.main === module) {
  setupTestFaucet();
}

module.exports = { setupTestFaucet };
