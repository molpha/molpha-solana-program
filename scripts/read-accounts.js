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

function formatAccountData(accountData, accountType) {
  const data = {};
  
  // Common fields
  if (accountData.authority) data.authority = accountData.authority.toString();
  if (accountData.bump) data.bump = accountData.bump;
  if (accountData.created_at) data.created_at = new Date(accountData.created_at * 1000).toISOString();
  
  // Account-specific fields
  switch (accountType) {
    case 'feed':
      data.name = accountData.name;
      data.feed_type = accountData.feed_type;
      data.job_id = Buffer.from(accountData.job_id).toString('hex');
      data.data_source = accountData.data_source.toString();
      data.min_signatures_threshold = accountData.min_signatures_threshold;
      data.frequency = accountData.frequency;
      data.ipfs_cid = accountData.ipfs_cid;
      data.subscription_due_time = new Date(accountData.subscription_due_time * 1000).toISOString();
      data.price_per_second_scaled = accountData.price_per_second_scaled;
      data.history_idx = accountData.history_idx;
      data.answer_history_count = accountData.answer_history.length;
      break;
      
    case 'dataSource':
      data.name = accountData.name;
      data.data_source_type = accountData.data_source_type;
      data.metadata_hash = `0x${Buffer.from(accountData.metadata_hash).toString('hex')}`;
      break;
      
    case 'protocolConfig':
      data.underlying_token = accountData.underlying_token.toString();
      data.base_price_per_second_scaled = accountData.base_price_per_second_scaled;
      data.frequency_coefficient = accountData.frequency_coefficient;
      data.signers_coefficient = accountData.signers_coefficient;
      data.reward_percentage = accountData.reward_percentage;
      data.priority_fee_buffer_percentage = accountData.priority_fee_buffer_percentage;
      data.max_priority_fee_coverage = accountData.max_priority_fee_coverage;
      data.priority_fee_smoothing_window = accountData.priority_fee_smoothing_window;
      break;
      
    case 'nodeRegistry':
      data.nodes_count = accountData.nodes.length;
      data.nodes = accountData.nodes.map(node => node.toString());
      break;
  }
  
  return data;
}

async function readAccount(program, accountAddress, accountType) {
  console.log(`🔍 Reading ${accountType} account: ${accountAddress}`);
  
  try {
    const accountPublicKey = new PublicKey(accountAddress);
    let accountData;
    
    // Fetch account based on type
    switch (accountType) {
      case 'feed':
        accountData = await program.account.feed.fetch(accountPublicKey);
        break;
      case 'dataSource':
        accountData = await program.account.dataSource.fetch(accountPublicKey);
        break;
      case 'protocolConfig':
        accountData = await program.account.protocolConfig.fetch(accountPublicKey);
        break;
      case 'nodeRegistry':
        accountData = await program.account.nodeRegistry.fetch(accountPublicKey);
        break;
      default:
        throw new Error(`Unknown account type: ${accountType}`);
    }
    
    const formattedData = formatAccountData(accountData, accountType);
    
    console.log("✅ Account Data:");
    console.log(JSON.stringify(formattedData, null, 2));
    
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(`❌ ${accountType} account not found: ${accountAddress}`);
    } else {
      console.error(`❌ Error reading ${accountType} account:`, error.message);
    }
    throw error;
  }
}

async function readAllAccounts(program) {
  console.log("🔍 Reading all program accounts...");
  
  try {
    const accounts = {
      feeds: [],
      dataSources: [],
      protocolConfig: null,
      nodeRegistry: null
    };
    
    // Read feeds
    try {
      const feeds = await program.account.feed.all();
      accounts.feeds = feeds.map(feed => ({
        address: feed.publicKey.toString(),
        name: feed.account.name,
        authority: feed.account.authority.toString(),
        feed_type: feed.account.feed_type
      }));
      console.log(`✅ Found ${feeds.length} feed(s)`);
    } catch (error) {
      console.log(`⚠️  Error reading feeds: ${error.message}`);
    }
    
    // Read data sources
    try {
      const dataSources = await program.account.dataSource.all();
      accounts.dataSources = dataSources.map(ds => ({
        address: ds.publicKey.toString(),
        name: ds.account.name,
        authority: ds.account.authority.toString(),
        data_source_type: ds.account.data_source_type
      }));
      console.log(`✅ Found ${dataSources.length} data source(s)`);
    } catch (error) {
      console.log(`⚠️  Error reading data sources: ${error.message}`);
    }
    
    // Read protocol config
    try {
      const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
        [Buffer.from("config")],
        program.programId
      );
      const protocolConfig = await program.account.protocolConfig.fetch(protocolConfigPDA);
      accounts.protocolConfig = {
        address: protocolConfigPDA.toString(),
        authority: protocolConfig.authority.toString(),
        underlying_token: protocolConfig.underlying_token.toString()
      };
      console.log(`✅ Found protocol config`);
    } catch (error) {
      console.log(`⚠️  Error reading protocol config: ${error.message}`);
    }
    
    // Read node registry
    try {
      const [nodeRegistryPDA] = anchor.web3.PublicKey.findProgramAddressSync(
        [Buffer.from("node-registry")],
        program.programId
      );
      const nodeRegistry = await program.account.nodeRegistry.fetch(nodeRegistryPDA);
      accounts.nodeRegistry = {
        address: nodeRegistryPDA.toString(),
        authority: nodeRegistry.authority.toString(),
        nodes_count: nodeRegistry.nodes.length
      };
      console.log(`✅ Found node registry with ${nodeRegistry.nodes.length} nodes`);
    } catch (error) {
      console.log(`⚠️  Error reading node registry: ${error.message}`);
    }
    
    console.log("");
    console.log("📊 Program Summary:");
    console.log(JSON.stringify(accounts, null, 2));
    
  } catch (error) {
    console.error("❌ Error reading accounts:", error.message);
    throw error;
  }
}

async function main() {
  setupEnvironment();

  const accountType = process.argv[2];
  const accountAddress = process.argv[3];

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    if (accountType && accountAddress) {
      await readAccount(program, accountAddress, accountType);
    } else if (!accountType) {
      await readAllAccounts(program);
    } else {
      console.error("❌ Invalid usage");
      console.log("Usage:");
      console.log("  node read-accounts.js                                    # Read all accounts");
      console.log("  node read-accounts.js <TYPE> <ADDRESS>                   # Read specific account");
      console.log("");
      console.log("Account types: feed, dataSource, protocolConfig, nodeRegistry");
      process.exit(1);
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
if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log("📖 Molpha Account Reader");
  console.log("");
  console.log("Usage:");
  console.log("  node read-accounts.js                                    # Read all accounts");
  console.log("  node read-accounts.js <TYPE> <ADDRESS>                   # Read specific account");
  console.log("  node read-accounts.js --help                             # Show this help");
  console.log("");
  console.log("Account Types:");
  console.log("  feed          - Feed account with subscription data");
  console.log("  dataSource    - Data source account");
  console.log("  protocolConfig - Protocol configuration account");
  console.log("  nodeRegistry  - Node registry account");
  console.log("");
  console.log("Examples:");
  console.log("  node read-accounts.js");
  console.log("  node read-accounts.js feed 7eb72Es6Z88LBzjTcypvi9uJXjy3FzNGc97X3DfhGZ7F");
  console.log("  node read-accounts.js dataSource 9K9FknHzW7j8a88yKTrzxKfDrxnV2QLqSR58ETAVdc8P");
  process.exit(0);
}

main().catch(console.error);
