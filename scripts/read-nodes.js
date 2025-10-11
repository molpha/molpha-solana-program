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

async function readNodeRegistry(program) {
  console.log("🔍 Reading node registry...");

  try {
    // Derive node registry PDA
    const [nodeRegistryPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node-registry")],
      program.programId
    );

    console.log(`Node Registry PDA: ${nodeRegistryPDA.toString()}`);
    console.log("");

    const nodeRegistry = await program.account.nodeRegistry.fetch(
      nodeRegistryPDA
    );

    console.log("✅ Node Registry:");
    console.log(`   Authority: ${nodeRegistry.authority.toString()}`);
    console.log(`   Total Nodes: ${nodeRegistry.nodes.length}`);
    console.log("");

    if (nodeRegistry.nodes.length === 0) {
      console.log("❌ No nodes registered");
      console.log("💡 Add nodes with: ./scripts/nodes add <NODE_PUBKEY>");
      return;
    }

    console.log("📋 Registered Nodes:");
    nodeRegistry.nodes.forEach((nodePubkey, index) => {
      console.log(`   ${index + 1}. ${nodePubkey.toString()}`);
    });
    console.log("");

    // Check node status (balance, etc.)
    console.log("🔍 Node Status Check:");
    for (let i = 0; i < nodeRegistry.nodes.length; i++) {
      const nodePubkey = nodeRegistry.nodes[i];
      try {
        const balance = await program.provider.connection.getBalance(
          nodePubkey
        );
        const solBalance = balance / 1e9;

        console.log(`   Node ${i + 1}: ${nodePubkey.toString()}`);
        console.log(`     Balance: ${solBalance.toFixed(4)} SOL`);

        if (solBalance < 0.01) {
          console.log(`     ⚠️  Low balance - may need funding`);
        } else if (solBalance < 0.1) {
          console.log(`     ⚠️  Moderate balance - consider monitoring`);
        } else {
          console.log(`     ✅ Good balance`);
        }
      } catch (error) {
        console.log(`   Node ${i + 1}: ${nodePubkey.toString()}`);
        console.log(`     ❌ Error checking balance: ${error.message}`);
      }
    }
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(
        "❌ Node registry not found. The protocol may not be initialized."
      );
      console.log(
        "💡 Initialize the protocol with: node scripts/initialize.js <TOKEN_MINT>"
      );
    } else {
      console.error("❌ Error reading node registry:", error.message);
    }
    throw error;
  }
}

async function checkSpecificNode(program, nodeAddress) {
  console.log(`🔍 Checking node: ${nodeAddress}`);

  try {
    const nodePublicKey = new PublicKey(nodeAddress);

    // Check if node is registered
    const [nodeRegistryPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node-registry")],
      program.programId
    );

    const nodeRegistry = await program.account.nodeRegistry.fetch(
      nodeRegistryPDA
    );
    const isRegistered = nodeRegistry.nodes.some((node) =>
      node.equals(nodePublicKey)
    );

    console.log("✅ Node Status:");
    console.log(`   Address: ${nodePublicKey.toString()}`);
    console.log(`   Registered: ${isRegistered ? "✅ Yes" : "❌ No"}`);

    if (isRegistered) {
      const index = nodeRegistry.nodes.findIndex((node) =>
        node.equals(nodePublicKey)
      );
      console.log(`   Index: ${index + 1} of ${nodeRegistry.nodes.length}`);
    }

    // Check balance
    try {
      const balance = await program.provider.connection.getBalance(
        nodePublicKey
      );
      const solBalance = balance / 1e9;

      console.log(`   Balance: ${solBalance.toFixed(4)} SOL`);

      if (solBalance < 0.01) {
        console.log(`   Status: ⚠️  Low balance - may need funding`);
      } else if (solBalance < 0.1) {
        console.log(`   Status: ⚠️  Moderate balance - consider monitoring`);
      } else {
        console.log(`   Status: ✅ Good balance`);
      }
    } catch (error) {
      console.log(`   Balance: ❌ Error checking balance: ${error.message}`);
    }
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(
        `❌ Node registry not found. The protocol may not be initialized.`
      );
    } else {
      console.error("❌ Error checking node:", error.message);
    }
    throw error;
  }
}

async function main() {
  setupEnvironment();

  const nodeAddress = process.argv[2];

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    if (nodeAddress) {
      await checkSpecificNode(program, nodeAddress);
    } else {
      await readNodeRegistry(program);
    }
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
  console.log("📖 Molpha Node Registry Reader");
  console.log("");
  console.log("Usage:");
  console.log(
    "  node read-nodes.js                    # Read all registered nodes"
  );
  console.log("  node read-nodes.js <NODE_ADDRESS>     # Check specific node");
  console.log("  node read-nodes.js --help             # Show this help");
  console.log("");
  console.log("This script displays:");
  console.log("  - All registered oracle nodes");
  console.log("  - Node balances and status");
  console.log("  - Registration status for specific nodes");
  console.log("");
  console.log("Examples:");
  console.log("  node read-nodes.js");
  console.log(
    "  node read-nodes.js 7eb72Es6Z88LBzjTcypvi9uJXjy3FzNGc97X3DfhGZ7F"
  );
  process.exit(0);
}

main().catch(console.error);
