#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { SystemProgram, PublicKey } = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
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

async function main() {
  // Parse command line arguments
  const underlyingTokenMint = process.argv[2]; // || "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"; // Default USDC
  console.log("Using underlying token mint:", underlyingTokenMint);

  if (!underlyingTokenMint) {
    console.error("❌ Error: Underlying token mint is required");
    console.log("Usage: node initialize.js <TOKEN_MINT_ADDRESS>");
    console.log(
      "Example: node initialize.js EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    );
    process.exit(1);
  }

  let underlyingToken;
  try {
    underlyingToken = new PublicKey(underlyingTokenMint);
  } catch (e) {
    console.error("❌ Error: Invalid token mint address");
    process.exit(1);
  }

  // Setup environment
  setupEnvironment();

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());

    // Derive PDAs
    const [nodeRegistryPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node-registry")],
      program.programId
    );

    const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId
    );

    // Derive program token account
    const programTokenAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      protocolConfigPDA,
      true // allowOwnerOffCurve
    );

    console.log("🔍 Initializing Molpha Protocol with:");
    console.log("Underlying token:", underlyingToken.toString());
    console.log("NodeRegistry PDA:", nodeRegistryPDA.toString());
    console.log("ProtocolConfig PDA:", protocolConfigPDA.toString());
    console.log("Program token account:", programTokenAccount.toString());
    console.log("Authority:", provider.wallet.publicKey.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    // Check if already initialized
    let nodeRegistryExists = false;
    let protocolConfigExists = false;

    try {
      await program.account.nodeRegistry.fetch(nodeRegistryPDA);
      nodeRegistryExists = true;
      console.log("⚠️  NodeRegistry already exists");
    } catch (e) {
      console.log("📝 NodeRegistry will be created");
    }

    try {
      await program.account.protocolConfig.fetch(protocolConfigPDA);
      protocolConfigExists = true;
      console.log("⚠️  ProtocolConfig already exists");
    } catch (e) {
      console.log("📝 ProtocolConfig will be created");
    }

    if (nodeRegistryExists && protocolConfigExists) {
      console.log("✅ Protocol is already fully initialized!");

      // Show current state
      const nodeRegistry = await program.account.nodeRegistry.fetch(
        nodeRegistryPDA
      );
      const protocolConfig = await program.account.protocolConfig.fetch(
        protocolConfigPDA
      );

      console.log("");
      console.log("📊 Current State:");
      console.log("NodeRegistry:");
      console.log("- Authority:", nodeRegistry.authority.toString());
      console.log("- Nodes count:", nodeRegistry.nodes.length);

      console.log("ProtocolConfig:");
      console.log("- Authority:", protocolConfig.authority.toString());
      console.log(
        "- Underlying token:",
        protocolConfig.underlyingToken.toString()
      );
      console.log(
        "- Base price per second:",
        protocolConfig.basePricePerSecondScaled.toString()
      );

      return;
    }

    console.log("🔄 Initializing protocol...");
    console.log("Underlying token:", underlyingToken.toString());
    const txSignature = await program.methods
      .initialize()
      .accounts({
        nodeRegistry: nodeRegistryPDA,
        protocolConfig: protocolConfigPDA,
        underlyingToken: underlyingToken,
        programTokenAccount: programTokenAccount,
        authority: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    console.log("✅ Protocol initialized successfully!");
    console.log("Transaction signature:", txSignature);
    console.log("");

    // Verify initialization
    try {
      const nodeRegistry = await program.account.nodeRegistry.fetch(
        nodeRegistryPDA
      );
      const protocolConfig = await program.account.protocolConfig.fetch(
        protocolConfigPDA
      );

      console.log("📊 Initialized State:");
      console.log("NodeRegistry:");
      console.log("- PDA:", nodeRegistryPDA.toString());
      console.log("- Authority:", nodeRegistry.authority.toString());
      console.log("- Nodes count:", nodeRegistry.nodes.length);

      console.log("ProtocolConfig:");
      console.log("- PDA:", protocolConfigPDA.toString());
      console.log("- Authority:", protocolConfig.authority.toString());
      console.log(
        "- Underlying token:",
        protocolConfig.underlyingToken.toString()
      );
      console.log("- Program token account:", programTokenAccount.toString());
      console.log(
        "- Base price per second:",
        protocolConfig.basePricePerSecondScaled.toString()
      );
      console.log(
        "- Reward percentage:",
        protocolConfig.rewardPercentage.toString(),
        "basis points"
      );

      console.log("");
      console.log("🎉 Protocol is ready for use!");
      console.log("Next steps:");
      console.log("1. Add nodes: ./scripts/nodes add <NODE_PUBKEY>");
      console.log("2. Create feeds: node scripts/create-feed.js");
      console.log("3. Start publishing data");
    } catch (e) {
      console.warn(
        "⚠️  Warning: Could not fetch initialized accounts for verification:",
        e.message
      );
    }
  } catch (e) {
    console.error("❌ Error initializing protocol:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs);
    }

    // Provide helpful error messages
    if (e.message.includes("failed to get recent blockhash")) {
      console.error("\n💡 Tip: Make sure the Solana validator is running:");
      console.error("   solana-test-validator --reset");
    } else if (
      e.message.includes(
        "Attempt to debit an account but found no record of a prior credit"
      )
    ) {
      console.error("\n💡 Tip: Make sure your wallet has sufficient SOL:");
      console.error("   solana airdrop 10");
    } else if (e.message.includes("already in use")) {
      console.error(
        "\n💡 Tip: Protocol may already be initialized. Check with:"
      );
      console.error("   ./scripts/nodes list");
    }

    process.exit(1);
  }
}

main().catch(console.error);
