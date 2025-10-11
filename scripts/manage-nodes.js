#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const fs = require("fs");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");

// Command line usage
const USAGE = `
Node Management Script for Molpha Protocol

Usage:
  node manage-nodes.js <command> [arguments]

Commands:
  check                             Check protocol initialization status
  register <node_pubkey> [evm_hex]  Register a single node (with optional EVM pubkey)
  batch-register <file_path>        Register multiple nodes from file
  status <node_pubkey>              Get status of a specific node
  generate-keypairs <count>         Generate keypairs for testing

Examples:
  node manage-nodes.js check
  node manage-nodes.js register 11111111111111111111111111111112
  node manage-nodes.js register 11111111111111111111111111111112 0x0404...
  node manage-nodes.js batch-register nodes.json
  node manage-nodes.js status 11111111111111111111111111111112
  node manage-nodes.js generate-keypairs 5

File format for batch-register (nodes.json):
[
  {
    "nodePubkey": "11111111111111111111111111111112",
    "evmPubkey": "0x0404..."
  },
  {
    "nodePubkey": "22222222222222222222222222222223"
  }
]

Or simple array format (uses default EVM pubkeys):
[
  "11111111111111111111111111111112",
  "22222222222222222222222222222223"
]
`;

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

async function setupProgram() {
  setupEnvironment();

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Molpha;

  const [nodeRegistryPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("node-registry")],
    program.programId
  );

  const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  return { program, provider, nodeRegistryPDA, protocolConfigPDA };
}

async function initializeProtocol(feePerUpdate = 1000) {
  const { program, provider, nodeRegistryPDA, protocolConfigPDA } =
    await setupProgram();

  try {
    console.log("🔍 Initializing Molpha Protocol with:");
    console.log("Fee per update:", feePerUpdate, "lamports");
    console.log("NodeRegistry PDA:", nodeRegistryPDA.toString());
    console.log("ProtocolConfig PDA:", protocolConfigPDA.toString());
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
        "- Fee per update:",
        protocolConfig.feePerUpdate.toString(),
        "lamports"
      );

      return true;
    }

    console.log("🔄 Initializing protocol...");

    const txSignature = await program.methods
      .initialize()
      .accounts({
        nodeRegistry: nodeRegistryPDA,
        protocolConfig: protocolConfigPDA,
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
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
        "- Fee per update:",
        protocolConfig.feePerUpdate.toString(),
        "lamports"
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

    return true;
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
      console.error("   ./scripts/nodes check");
    }

    return false;
  }
}

async function checkProtocolStatus() {
  const { program, nodeRegistryPDA, protocolConfigPDA } = await setupProgram();

  try {
    console.log("🔍 Checking Protocol Status:");
    console.log("NodeRegistry PDA:", nodeRegistryPDA.toString());
    console.log("ProtocolConfig PDA:", protocolConfigPDA.toString());
    console.log("");

    let nodeRegistryExists = false;
    let protocolConfigExists = false;
    let nodeRegistry = null;
    let protocolConfig = null;

    // Check NodeRegistry
    try {
      nodeRegistry = await program.account.nodeRegistry.fetch(nodeRegistryPDA);
      nodeRegistryExists = true;
      console.log("✅ NodeRegistry: Initialized");
      console.log("   Authority:", nodeRegistry.authority.toString());
      console.log("   Nodes count:", nodeRegistry.nodes.length);
    } catch (e) {
      console.log("❌ NodeRegistry: Not initialized");
    }

    // Check ProtocolConfig
    try {
      protocolConfig = await program.account.protocolConfig.fetch(
        protocolConfigPDA
      );
      protocolConfigExists = true;
      console.log("✅ ProtocolConfig: Initialized");
      console.log("   Authority:", protocolConfig.authority.toString());
      console.log(
        "   Fee per update:",
        protocolConfig.feePerUpdate.toString(),
        "lamports"
      );
    } catch (e) {
      console.log("❌ ProtocolConfig: Not initialized");
    }

    console.log("");

    if (nodeRegistryExists && protocolConfigExists) {
      console.log("🎉 Protocol is fully initialized and ready to use!");

      if (nodeRegistry.nodes.length === 0) {
        console.log(
          "💡 Next step: Add some nodes with './scripts/nodes add <NODE_PUBKEY>'"
        );
      }
    } else {
      console.log("⚠️  Protocol is not fully initialized.");
      console.log("💡 Run: './scripts/nodes init' to initialize the protocol");
    }
  } catch (e) {
    console.error("❌ Error checking protocol status:", e.message);

    if (e.message.includes("failed to get recent blockhash")) {
      console.error("\n💡 Tip: Make sure the Solana validator is running:");
      console.error("   solana-test-validator --reset");
    }
  }
}

async function registerNode(nodePubkeyStr, evmPubkeyHex = null) {
  const { program, provider, protocolConfigPDA } = await setupProgram();

  try {
    const nodePubkey = new anchor.web3.PublicKey(nodePubkeyStr);

    // Parse EVM pubkey or use default
    const hexString = evmPubkeyHex.startsWith("0x")
      ? evmPubkeyHex.slice(2)
      : evmPubkeyHex;
    let evmPubkey = Buffer.from(hexString, "hex");
    if (evmPubkey.length !== 33) {
      console.error(
        "❌ Error: EVM pubkey must be 33 bytes (compressed secp256k1)"
      );
      return false;
    }

    const [nodePDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node"), nodePubkey.toBuffer()],
      program.programId
    );

    // Check if protocol config exists
    let protocolConfig;
    try {
      protocolConfig = await program.account.protocolConfig.fetch(
        protocolConfigPDA
      );
    } catch (e) {
      console.error(
        "❌ Error: Protocol config not found. Please run initialize script first."
      );
      return false;
    }

    // Get underlying token and security deposit
    const underlyingToken = protocolConfig.underlyingToken;
    const securityDeposit = protocolConfig.nodeSecurityDeposit;

    // Get operator's token account
    const nodeOperatorTokenAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      provider.wallet.publicKey
    );

    // Get node's security deposit account
    const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      nodePDA,
      true // allowOwnerOffCurve
    );

    // Check if node already exists
    try {
      await program.account.node.fetch(nodePDA);
      console.error(`❌ Error: Node ${nodePubkeyStr} already exists`);
      return false;
    } catch (e) {
      // Node doesn't exist, which is good for registering
    }

    console.log(`🔄 Registering node: ${nodePubkeyStr}`);
    console.log(`   Security deposit: ${securityDeposit.toString()} tokens`);

    const txSignature = await program.methods
      .registerNode(nodePubkey, evmPubkey)
      .accounts({
        node: nodePDA,
        protocolConfig: protocolConfigPDA,
        underlyingToken: underlyingToken,
        nodeOperatorTokenAccount: nodeOperatorTokenAccount,
        nodeSecurityDepositAccount: nodeSecurityDepositAccount,
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    console.log(`✅ Node registered successfully: ${nodePubkeyStr}`);
    console.log(`   Transaction: ${txSignature}`);
    return true;
  } catch (e) {
    console.error(`❌ Error registering node ${nodePubkeyStr}:`, e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs.join("\n"));
    }
    return false;
  }
}

async function removeNode(nodePubkeyStr) {
  const { program, provider, nodeRegistryPDA } = await setupProgram();

  try {
    const nodePubkey = new anchor.web3.PublicKey(nodePubkeyStr);

    const [nodePDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node"), nodePubkey.toBuffer()],
      program.programId
    );

    // Check if node exists
    try {
      await program.account.node.fetch(nodePDA);
    } catch (e) {
      console.error(`❌ Error: Node ${nodePubkeyStr} not found`);
      return false;
    }

    console.log(`🔄 Removing node: ${nodePubkeyStr}`);

    const txSignature = await program.methods
      .removeNode(nodePubkey)
      .accounts({
        nodeRegistry: nodeRegistryPDA,
        node: nodePDA,
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    console.log(`✅ Node removed successfully: ${nodePubkeyStr}`);
    console.log(`   Transaction: ${txSignature}`);
    return true;
  } catch (e) {
    console.error(`❌ Error removing node ${nodePubkeyStr}:`, e.message);
    return false;
  }
}

async function batchRegisterNodes(filePath) {
  if (!fs.existsSync(filePath)) {
    console.error(`❌ Error: File not found: ${filePath}`);
    return;
  }

  let nodes;
  try {
    const fileContent = fs.readFileSync(filePath, "utf8");
    nodes = JSON.parse(fileContent);
  } catch (e) {
    console.error(`❌ Error reading file ${filePath}:`, e.message);
    return;
  }

  if (!Array.isArray(nodes)) {
    console.error("❌ Error: File must contain an array of node entries");
    return;
  }

  console.log(`📋 Batch registering ${nodes.length} nodes...`);

  let successCount = 0;
  let failureCount = 0;

  for (let i = 0; i < nodes.length; i++) {
    const nodeEntry = nodes[i];
    console.log(`\n[${i + 1}/${nodes.length}]`);

    let nodePubkeyStr, evmPubkeyHex;

    // Support both string format and object format
    if (typeof nodeEntry === "string") {
      nodePubkeyStr = nodeEntry;
      evmPubkeyHex = null;
    } else if (typeof nodeEntry === "object") {
      nodePubkeyStr = nodeEntry.nodePubkey;
      evmPubkeyHex = nodeEntry.evmPubkey || null;
    } else {
      console.error(`❌ Invalid node entry format at index ${i}`);
      failureCount++;
      continue;
    }

    const success = await registerNode(nodePubkeyStr, evmPubkeyHex);
    if (success) {
      successCount++;
    } else {
      failureCount++;
    }
  }

  console.log(`\n📊 Batch operation completed:`);
  console.log(`   ✅ Successful: ${successCount}`);
  console.log(`   ❌ Failed: ${failureCount}`);
  console.log(`   📋 Total: ${nodes.length}`);
}

async function listNodes() {
  const { program, nodeRegistryPDA } = await setupProgram();

  try {
    const nodeRegistryAccount = await program.account.nodeRegistry.fetch(
      nodeRegistryPDA
    );

    console.log(`📋 Node Registry Status:`);
    console.log(`   Authority: ${nodeRegistryAccount.authority.toString()}`);
    console.log(`   Total Nodes: ${nodeRegistryAccount.nodes.length}`);
    console.log("");

    if (nodeRegistryAccount.nodes.length === 0) {
      console.log("   No nodes registered");
      return;
    }

    console.log("📊 Registered Nodes:");
    for (let i = 0; i < nodeRegistryAccount.nodes.length; i++) {
      const nodePubkey = nodeRegistryAccount.nodes[i];
      console.log(`   ${i + 1}. ${nodePubkey.toString()}`);

      // Try to get detailed node information
      try {
        const [nodePDA] = anchor.web3.PublicKey.findProgramAddressSync(
          [Buffer.from("node"), nodePubkey.toBuffer()],
          program.programId
        );

        const nodeAccount = await program.account.node.fetch(nodePDA);
        console.log(`      - Active: ${nodeAccount.isActive}`);
        console.log(
          `      - Created: ${new Date(
            nodeAccount.createdAt.toNumber() * 1000
          ).toISOString()}`
        );
        console.log(
          `      - Last Active: ${new Date(
            nodeAccount.lastActive.toNumber() * 1000
          ).toISOString()}`
        );
      } catch (e) {
        console.log(`      - Details: Unable to fetch (${e.message})`);
      }
    }
  } catch (e) {
    console.log("nodeRegistryPDA", nodeRegistryPDA.toString());

    console.log("e", e);
    console.error(
      "❌ Error: Node registry not found. Please run initialize script first."
    );
  }
}

async function getNodeStatus(nodePubkeyStr) {
  const { program } = await setupProgram();

  try {
    const nodePubkey = new anchor.web3.PublicKey(nodePubkeyStr);

    const [nodePDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node"), nodePubkey.toBuffer()],
      program.programId
    );

    const nodeAccount = await program.account.node.fetch(nodePDA);

    console.log(`📊 Node Status: ${nodePubkeyStr}`);
    console.log(`   PDA Address: ${nodePDA.toString()}`);
    console.log(`   Authority: ${nodeAccount.authority.toString()}`);
    console.log(`   Node Pubkey: ${nodeAccount.nodePubkey.toString()}`);
    console.log(`   Is Active: ${nodeAccount.isActive}`);
    console.log(
      `   Created At: ${new Date(
        nodeAccount.createdAt.toNumber() * 1000
      ).toISOString()}`
    );
    console.log(
      `   Last Active: ${new Date(
        nodeAccount.lastActive.toNumber() * 1000
      ).toISOString()}`
    );
  } catch (e) {
    console.error(
      `❌ Error: Node ${nodePubkeyStr} not found or unable to fetch details`
    );
  }
}

async function generateKeypairs(count) {
  console.log(`🔑 Generating ${count} keypairs for testing...`);

  const keypairs = [];
  for (let i = 0; i < count; i++) {
    const keypair = anchor.web3.Keypair.generate();
    keypairs.push({
      publicKey: keypair.publicKey.toString(),
      secretKey: Array.from(keypair.secretKey),
    });

    console.log(`   ${i + 1}. ${keypair.publicKey.toString()}`);
  }

  const filename = `generated-nodes-${Date.now()}.json`;
  const publicKeys = keypairs.map((kp) => kp.publicKey);

  // Save public keys for batch adding
  fs.writeFileSync(filename, JSON.stringify(publicKeys, null, 2));

  // Save full keypairs for reference
  const fullFilename = `generated-keypairs-${Date.now()}.json`;
  fs.writeFileSync(fullFilename, JSON.stringify(keypairs, null, 2));

  console.log(`\n💾 Files saved:`);
  console.log(`   Public keys (for batch-register): ${filename}`);
  console.log(`   Full keypairs: ${fullFilename}`);
  console.log(`\n📋 To register these nodes, run:`);
  console.log(`   node manage-nodes.js batch-register ${filename}`);
}

async function main() {
  const command = process.argv[2];
  const arg = process.argv[3];

  if (!command) {
    console.log(USAGE);
    return;
  }

  switch (command.toLowerCase()) {
    case "check":
      await checkProtocolStatus();
      break;

    case "register":
      if (!arg) {
        console.error("❌ Error: Node public key is required");
        console.log(
          "Usage: node manage-nodes.js register <NODE_PUBKEY> [EVM_PUBKEY_HEX]"
        );
        return;
      }
      const evmPubkey = process.argv[4] || null;
      await registerNode(arg, evmPubkey);
      break;

    case "batch-register":
      if (!arg) {
        console.error("❌ Error: File path is required");
        console.log("Usage: node manage-nodes.js batch-register <FILE_PATH>");
        return;
      }
      await batchRegisterNodes(arg);
      break;

    case "status":
      if (!arg) {
        console.error("❌ Error: Node public key is required");
        console.log("Usage: node manage-nodes.js status <NODE_PUBKEY>");
        return;
      }
      await getNodeStatus(arg);
      break;

    case "generate-keypairs":
      const count = parseInt(arg) || 5;
      await generateKeypairs(count);
      break;

    default:
      console.error(`❌ Error: Unknown command '${command}'`);
      console.log(USAGE);
      break;
  }
}

main().catch(console.error);
