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

function formatDataSourceType(dataSourceType) {
  if (!dataSourceType) return "Unknown";
  if (dataSourceType.public !== undefined) return "Public";
  if (dataSourceType.private !== undefined) return "Private";
  return "Unknown";
}

function formatMetadataHash(metadataHash) {
  if (!metadataHash) return "No metadata hash";
  return `0x${Buffer.from(metadataHash).toString("hex")}`;
}

async function readAllDataSources(program) {
  console.log("🔍 Searching for all data sources...");

  try {
    // Get all data source accounts
    const dataSources = await program.account.dataSource.all();

    if (dataSources.length === 0) {
      console.log("❌ No data sources found");
      return;
    }

    console.log(`✅ Found ${dataSources.length} data source(s):`);
    console.log("");

    dataSources.forEach((dataSourceAccount, index) => {
      const dataSource = dataSourceAccount.account;

      console.log(`📊 Data Source #${index + 1}`);
      console.log(`   Address: ${dataSourceAccount.publicKey.toString()}`);
      console.log(`   Name: ${dataSource.name}`);
      console.log(`   Authority: ${dataSource.authority.toString()}`);
      console.log(
        `   Type: ${formatDataSourceType(dataSource.data_source_type)}`
      );
      console.log(
        `   Metadata Hash: ${formatMetadataHash(dataSource.metadata_hash)}`
      );
      // Handle created_at safely
      let createdAt;
      if (dataSource.created_at && dataSource.created_at > 0) {
        try {
          createdAt = new Date(dataSource.created_at * 1000).toISOString();
        } catch (e) {
          createdAt = "Invalid creation time";
        }
      } else {
        createdAt = "No creation time";
      }

      console.log(`   Created: ${createdAt}`);
      console.log(`   Bump: ${dataSource.bump || "Not set"}`);
      console.log("");
    });
  } catch (error) {
    console.error("❌ Error reading data sources:", error.message);
    throw error;
  }
}

async function readSpecificDataSource(program, dataSourceAddress) {
  console.log(`🔍 Reading data source: ${dataSourceAddress}`);

  try {
    const dataSourcePublicKey = new PublicKey(dataSourceAddress);
    const dataSource = await program.account.dataSource.fetch(
      dataSourcePublicKey
    );

    console.log("✅ Data Source Details:");
    console.log(`   Address: ${dataSourcePublicKey.toString()}`);
    console.log(`   Name: ${dataSource.name}`);
    console.log(`   Authority: ${dataSource.authority.toString()}`);
    console.log(
      `   Type: ${formatDataSourceType(dataSource.data_source_type)}`
    );
    console.log(
      `   Metadata Hash: ${formatMetadataHash(dataSource.metadata_hash)}`
    );
    console.log(
      `   Created: ${new Date(dataSource.created_at * 1000).toISOString()}`
    );
    console.log(`   Bump: ${dataSource.bump}`);
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(`❌ Data source not found: ${dataSourceAddress}`);
    } else {
      console.error("❌ Error reading data source:", error.message);
    }
    throw error;
  }
}

async function readDataSourcesByAuthority(program, authorityAddress) {
  console.log(
    `🔍 Searching for data sources by authority: ${authorityAddress}`
  );

  try {
    const authorityPublicKey = new PublicKey(authorityAddress);
    const dataSources = await program.account.dataSource.all();

    const filteredDataSources = dataSources.filter((ds) =>
      ds.account.authority.equals(authorityPublicKey)
    );

    if (filteredDataSources.length === 0) {
      console.log(
        `❌ No data sources found for authority: ${authorityAddress}`
      );
      return;
    }

    console.log(
      `✅ Found ${filteredDataSources.length} data source(s) for authority ${authorityAddress}:`
    );
    console.log("");

    filteredDataSources.forEach((dataSourceAccount, index) => {
      const dataSource = dataSourceAccount.account;

      console.log(`📊 Data Source #${index + 1}`);
      console.log(`   Address: ${dataSourceAccount.publicKey.toString()}`);
      console.log(`   Name: ${dataSource.name}`);
      console.log(`   Authority: ${dataSource.authority.toString()}`);
      console.log(
        `   Type: ${formatDataSourceType(dataSource.data_source_type)}`
      );
      console.log(
        `   Metadata Hash: ${formatMetadataHash(dataSource.metadata_hash)}`
      );
      // Handle created_at safely
      let createdAt;
      if (dataSource.created_at && dataSource.created_at > 0) {
        try {
          createdAt = new Date(dataSource.created_at * 1000).toISOString();
        } catch (e) {
          createdAt = "Invalid creation time";
        }
      } else {
        createdAt = "No creation time";
      }

      console.log(`   Created: ${createdAt}`);
      console.log(`   Bump: ${dataSource.bump || "Not set"}`);
      console.log("");
    });
  } catch (error) {
    console.error("❌ Error reading data sources by authority:", error.message);
    throw error;
  }
}

async function readDataSourcesByType(program, type) {
  const dataSourceType = type.toLowerCase();
  if (!["public", "private"].includes(dataSourceType)) {
    console.error("❌ Invalid data source type. Must be 'public' or 'private'");
    return;
  }

  console.log(`🔍 Searching for ${dataSourceType} data sources...`);

  try {
    const dataSources = await program.account.dataSource.all();

    const filteredDataSources = dataSources.filter((ds) => {
      const dsType = formatDataSourceType(
        ds.account.data_source_type
      ).toLowerCase();
      return dsType === dataSourceType;
    });

    if (filteredDataSources.length === 0) {
      console.log(`❌ No ${dataSourceType} data sources found`);
      return;
    }

    console.log(
      `✅ Found ${filteredDataSources.length} ${dataSourceType} data source(s):`
    );
    console.log("");

    filteredDataSources.forEach((dataSourceAccount, index) => {
      const dataSource = dataSourceAccount.account;

      console.log(`📊 Data Source #${index + 1}`);
      console.log(`   Address: ${dataSourceAccount.publicKey.toString()}`);
      console.log(`   Name: ${dataSource.name}`);
      console.log(`   Authority: ${dataSource.authority.toString()}`);
      console.log(
        `   Type: ${formatDataSourceType(dataSource.data_source_type)}`
      );
      console.log(
        `   Metadata Hash: ${formatMetadataHash(dataSource.metadata_hash)}`
      );
      // Handle created_at safely
      let createdAt;
      if (dataSource.created_at && dataSource.created_at > 0) {
        try {
          createdAt = new Date(dataSource.created_at * 1000).toISOString();
        } catch (e) {
          createdAt = "Invalid creation time";
        }
      } else {
        createdAt = "No creation time";
      }

      console.log(`   Created: ${createdAt}`);
      console.log(`   Bump: ${dataSource.bump || "Not set"}`);
      console.log("");
    });
  } catch (error) {
    console.error("❌ Error reading data sources by type:", error.message);
    throw error;
  }
}

async function main() {
  setupEnvironment();

  const command = process.argv[2];
  const param = process.argv[3];

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    if (command === "address" && param) {
      await readSpecificDataSource(program, param);
    } else if (command === "authority" && param) {
      await readDataSourcesByAuthority(program, param);
    } else if (command === "type" && param) {
      await readDataSourcesByType(program, param);
    } else if (!command) {
      await readAllDataSources(program);
    } else {
      console.error("❌ Invalid command or missing parameter");
      console.log("Usage:");
      console.log(
        "  node read-data-sources.js                    # Read all data sources"
      );
      console.log(
        "  node read-data-sources.js address <ADDRESS>   # Read specific data source"
      );
      console.log(
        "  node read-data-sources.js authority <ADDRESS> # Read by authority"
      );
      console.log(
        "  node read-data-sources.js type <public|private> # Read by type"
      );
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
if (process.argv.includes("--help") || process.argv.includes("-h")) {
  console.log("📖 Molpha Data Source Reader");
  console.log("");
  console.log("Usage:");
  console.log(
    "  node read-data-sources.js                                    # Read all data sources"
  );
  console.log(
    "  node read-data-sources.js address <ADDRESS>                  # Read specific data source"
  );
  console.log(
    "  node read-data-sources.js authority <AUTHORITY_ADDRESS>      # Read by authority"
  );
  console.log(
    "  node read-data-sources.js type <public|private>              # Read by type"
  );
  console.log(
    "  node read-data-sources.js --help                             # Show this help"
  );
  console.log("");
  console.log("Examples:");
  console.log("  node read-data-sources.js");
  console.log(
    "  node read-data-sources.js address 7eb72Es6Z88LBzjTcypvi9uJXjy3FzNGc97X3DfhGZ7F"
  );
  console.log(
    "  node read-data-sources.js authority 9K9FknHzW7j8a88yKTrzxKfDrxnV2QLqSR58ETAVdc8P"
  );
  console.log("  node read-data-sources.js type public");
  process.exit(0);
}

main().catch(console.error);
