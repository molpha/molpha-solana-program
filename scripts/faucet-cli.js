#!/usr/bin/env node

const { Command } = require("commander");
const { Keypair, PublicKey } = require("@solana/web3.js");
const fs = require("fs");
const {
  initializeFaucet,
  requestTokens,
  getFaucetInfo,
  getUserCooldown,
  getUserTokenBalance,
  saveTokenMintKeypair,
  loadTokenMintKeypair,
} = require("./faucet-utils");

const program = new Command();

// Helper function to load keypair from file
function loadKeypair(filepath) {
  try {
    const secretKeyString = fs.readFileSync(filepath, "utf8");
    const secretKey = Uint8Array.from(JSON.parse(secretKeyString));
    return Keypair.fromSecretKey(secretKey);
  } catch (error) {
    console.error(`❌ Error loading keypair from ${filepath}:`, error.message);
    process.exit(1);
  }
}

// Helper function to parse PublicKey
function parsePublicKey(pubkeyString) {
  try {
    return new PublicKey(pubkeyString);
  } catch (error) {
    console.error(`❌ Invalid public key: ${pubkeyString}`);
    process.exit(1);
  }
}

program
  .name("faucet-cli")
  .description("CLI tool for interacting with the Solana Token Faucet")
  .version("1.0.0");

// Initialize faucet command
program
  .command("init")
  .description("Initialize a new faucet with a new token mint")
  .requiredOption("-a, --authority <path>", "Path to authority keypair file")
  .option("-r, --amount <number>", "Amount of tokens per request", "1000")
  .option("-c, --cooldown <number>", "Cooldown period in seconds", "300")
  .option("-d, --decimals <number>", "Token decimals", "6")
  .option("-n, --name <string>", "Token name", "Test Token")
  .option("-s, --symbol <string>", "Token symbol", "TEST")
  .option("--save-mint <path>", "Path to save token mint keypair")
  .action(async (options) => {
    try {
      console.log("🚀 Initializing faucet...\n");

      const authority = loadKeypair(options.authority);
      const amountPerRequest = parseInt(options.amount);
      const cooldownSeconds = parseInt(options.cooldown);
      const tokenDecimals = parseInt(options.decimals);

      const result = await initializeFaucet(
        authority,
        amountPerRequest,
        cooldownSeconds,
        tokenDecimals,
        options.name,
        options.symbol
      );

      if (options.saveMint) {
        saveTokenMintKeypair(result.tokenMintKeypair, options.saveMint);
      }

      console.log("\n🎉 Faucet initialized successfully!");
      console.log("\n📋 Save these details:");
      console.log(`Token Mint: ${result.tokenMint.toString()}`);
      console.log(`Faucet Config: ${result.faucetConfigPDA.toString()}`);
    } catch (error) {
      console.error("❌ Failed to initialize faucet:", error.message);
      process.exit(1);
    }
  });

// Request tokens command
program
  .command("request")
  .description("Request tokens from the faucet")
  .requiredOption("-m, --mint <pubkey>", "Token mint public key")
  .requiredOption("-u, --user <path>", "Path to user keypair file")
  .action(async (options) => {
    try {
      console.log("💧 Requesting tokens from faucet...\n");

      const tokenMint = parsePublicKey(options.mint);
      const user = loadKeypair(options.user);

      await requestTokens(tokenMint, user);

      console.log("\n🎉 Token request completed!");
    } catch (error) {
      console.error("❌ Failed to request tokens:", error.message);
      process.exit(1);
    }
  });

// Get faucet info command
program
  .command("info")
  .description("Get faucet information")
  .requiredOption("-m, --mint <pubkey>", "Token mint public key")
  .action(async (options) => {
    try {
      const tokenMint = parsePublicKey(options.mint);
      await getFaucetInfo(tokenMint);
    } catch (error) {
      console.error("❌ Failed to get faucet info:", error.message);
      process.exit(1);
    }
  });

// Get user cooldown command
program
  .command("cooldown")
  .description("Check user cooldown status")
  .requiredOption("-m, --mint <pubkey>", "Token mint public key")
  .requiredOption("-u, --user <pubkey>", "User public key")
  .action(async (options) => {
    try {
      const tokenMint = parsePublicKey(options.mint);
      const userPubkey = parsePublicKey(options.user);
      await getUserCooldown(tokenMint, userPubkey);
    } catch (error) {
      console.error("❌ Failed to get cooldown info:", error.message);
      process.exit(1);
    }
  });

// Get user balance command
program
  .command("balance")
  .description("Check user token balance")
  .requiredOption("-m, --mint <pubkey>", "Token mint public key")
  .requiredOption("-u, --user <pubkey>", "User public key")
  .action(async (options) => {
    try {
      const tokenMint = parsePublicKey(options.mint);
      const userPubkey = parsePublicKey(options.user);
      await getUserTokenBalance(tokenMint, userPubkey);
    } catch (error) {
      console.error("❌ Failed to get balance:", error.message);
      process.exit(1);
    }
  });

// Load mint keypair command
program
  .command("load-mint")
  .description("Load token mint keypair from file and display public key")
  .requiredOption("-f, --file <path>", "Path to token mint keypair file")
  .action(async (options) => {
    try {
      const keypair = loadTokenMintKeypair(options.file);
      console.log("✅ Token mint public key:", keypair.publicKey.toString());
    } catch (error) {
      console.error("❌ Failed to load mint keypair:", error.message);
      process.exit(1);
    }
  });

// Parse command line arguments
program.parse();
