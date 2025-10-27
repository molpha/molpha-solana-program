#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { PublicKey } = require("@solana/web3.js");
const fs = require("fs");

// Setup environment variables if not set
function setupEnvironment() {
  if (!process.env.ANCHOR_PROVIDER_URL) {
    process.env.ANCHOR_PROVIDER_URL = "http://127.0.0.1:8899";
  }

  if (!process.env.ANCHOR_WALLET) {
    const defaultWallet = require("os").homedir() + "/.config/solana/id.json";
    if (fs.existsSync(defaultWallet)) {
      process.env.ANCHOR_WALLET = defaultWallet;
    }
  }
}

function formatTokenAmount(amount, decimals = 9) {
  return (amount / Math.pow(10, decimals)).toFixed(decimals);
}

function formatBasisPoints(basisPoints) {
  return `${basisPoints} bp (${(basisPoints / 100).toFixed(2)}%)`;
}

async function readProtocolConfig(program) {
  console.log("🔍 Reading protocol configuration...");

  try {
    // Derive protocol config PDA
    const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId
    );

    console.log(`Protocol Config PDA: ${protocolConfigPDA.toString()}`);
    console.log("");

    const config = await program.account.protocolConfig.fetch(
      protocolConfigPDA
    );

    console.log("✅ Protocol Configuration:");
    console.log(
      `   Authority: ${
        config.authority ? config.authority.toString() : "Not set"
      }`
    );
    console.log(
      `   Underlying Token: ${
        config.underlyingToken ? config.underlyingToken.toString() : "Not set"
      }`
    );
    console.log(`   Bump: ${config.bump || "Not set"}`);
    console.log("");

    console.log("💰 Pricing Configuration:");
    console.log(
      `   Base Price per Second (scaled): ${
        config.basePricePerSecondScaled
          ? config.basePricePerSecondScaled.toLocaleString()
          : "Not set"
      }`
    );
    console.log(
      `   Base Price per Second (actual): ${
        config.basePricePerSecondScaled
          ? formatTokenAmount(config.basePricePerSecondScaled / 1000000)
          : "Not set"
      } tokens`
    );
    console.log(
      `   Frequency Coefficient: ${
        config.frequencyCoefficient
          ? formatBasisPoints(config.frequencyCoefficient)
          : "Not set"
      }`
    );
    console.log(
      `   Signers Coefficient: ${
        config.signersCoefficient
          ? formatBasisPoints(config.signersCoefficient)
          : "Not set"
      }`
    );
    console.log(
      `   Reward Percentage: ${
        config.rewardPercentage
          ? formatBasisPoints(config.rewardPercentage)
          : "Not set"
      }`
    );
    console.log("");

    console.log("⚡ Priority Fee Configuration:");
    console.log(
      `   Priority Fee Buffer: ${
        config.priorityFeeBufferPercentage || "Not set"
      }%`
    );
    console.log(
      `   Max Priority Fee Coverage: ${
        config.maxPriorityFeeCoverage
          ? config.maxPriorityFeeCoverage.toLocaleString()
          : "Not set"
      } lamports`
    );
    console.log(
      `   Priority Fee Smoothing Window: ${
        config.priorityFeeSmoothingWindow || "Not set"
      } transactions`
    );
    console.log("");

    // Calculate some example pricing scenarios
    console.log("📊 Example Pricing Scenarios:");
    console.log("   (Based on current configuration)");
    console.log("");

    // Check if program token account exists
    try {
      const { getAssociatedTokenAddressSync } = require("@solana/spl-token");
      const programTokenAccount = getAssociatedTokenAddressSync(
        config.underlying_token,
        protocolConfigPDA,
        true // allowOwnerOffCurve
      );

      console.log("🏦 Program Token Account:");
      console.log(`   Address: ${programTokenAccount.toString()}`);

      // Try to get balance
      try {
        const tokenAccount =
          await program.provider.connection.getTokenAccountBalance(
            programTokenAccount
          );
        console.log(
          `   Balance: ${formatTokenAmount(
            parseInt(tokenAccount.value.amount),
            tokenAccount.value.decimals
          )} tokens`
        );
      } catch (e) {
        console.log(`   Balance: Account not found or not initialized`);
      }
    } catch (e) {
      console.log(`   Token Account: Error calculating address`);
    }
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(
        "❌ Protocol configuration not found. The protocol may not be initialized."
      );
      console.log(
        "💡 Initialize the protocol with: node scripts/initialize.js <TOKEN_MINT>"
      );
    } else {
      console.error("❌ Error reading protocol configuration:", error.message);
    }
    throw error;
  }
}

async function main() {
  setupEnvironment();

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    await readProtocolConfig(program);
  } catch (error) {
    console.error("❌ Error:", error.message);
    if (error.logs) {
      console.error("Transaction logs:", error.logs);
    }
    process.exit(1);
  }
}

// Show usage if help requested
if (process.argv.includes("--help") || process.argv.includes("-h")) {
  console.log("📖 Molpha Protocol Configuration Reader");
  console.log("");
  console.log("Usage:");
  console.log(
    "  node read-protocol-config.js              # Read protocol configuration"
  );
  console.log("  node read-protocol-config.js --help       # Show this help");
  console.log("");
  console.log("This script displays:");
  console.log("  - Protocol authority and underlying token");
  console.log("  - Pricing parameters and coefficients");
  console.log("  - Priority fee configuration");
  console.log("  - Example pricing scenarios");
  console.log("  - Program token account balance");
  process.exit(0);
}

main().catch(console.error);
