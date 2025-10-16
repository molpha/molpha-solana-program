#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { SystemProgram } = require("@solana/web3.js");
const { getDataSourcePda } = require("./pda-helpers");
const crypto = require("crypto");

async function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Data source name is required");
    console.log(
      "Usage: node create-data-source.js <DATA_SOURCE_NAME> [TYPE]"
    );
    console.log('  DATA_SOURCE_NAME: name of the data source (will be hashed for PDA)');
    console.log('  TYPE: "personal" or "public" (default: public)');
    process.exit(1);
  }

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Molpha;

  // Parse command line arguments
  const dataSourceName = process.argv[2];
  const dataSourceTypeStr = process.argv[3] || "public";
  const dataSourceType = dataSourceTypeStr === "personal" ? { personal: {} } : { public: {} };
  const dataSourceTypeNum = dataSourceTypeStr === "personal" ? 1 : 0;

  // Generate a random metadata hash (32 bytes)
  const metadataHash = Array.from(crypto.randomBytes(32));

  // Derive data source PDA
  const [dataSourcePDA] = getDataSourcePda(
    program.programId,
    provider.wallet.publicKey,
    dataSourceName,
    dataSourceTypeNum
  );

  try {
    console.log("🔄 Creating data source...");
    console.log("Data source name:", dataSourceName);
    console.log("Data source PDA:", dataSourcePDA.toString());

    const txSignature = await program.methods
      .createDataSource({
        name: dataSourceName,
        dataSourceType: dataSourceType,
        metadataHash: metadataHash,
      })
      .accounts({
        authority: provider.wallet.publicKey,
        dataSource: dataSourcePDA,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("✅ Data source created successfully!");
    console.log("Data source name:", dataSourceName);
    console.log("Data source type:", dataSourceTypeStr);
    console.log("Data source PDA:", dataSourcePDA.toString());
    console.log("Transaction signature:", txSignature);
  } catch (e) {
    console.error("❌ Error creating data source:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs.join("\n"));
    }
    process.exit(1);
  }
}

main().catch(console.error);

