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

function parseBasisPoints(value) {
  if (typeof value === "string" && value.includes("%")) {
    return Math.round(parseFloat(value.replace("%", "")) * 100);
  }
  return new anchor.BN(parseInt(value));
}

function parseTokenAmount(value, decimals = 9) {
  if (typeof value === "string" && value.includes(".")) {
    return Math.round(parseFloat(value) * Math.pow(10, decimals));
  }
  return new anchor.BN(parseInt(value));
}

function formatBasisPoints(basisPoints) {
  return `${basisPoints} bp (${(basisPoints / 100).toFixed(2)}%)`;
}

function formatTokenAmount(amount, decimals = 9) {
  return (amount / Math.pow(10, decimals)).toFixed(decimals);
}

async function updateProtocolConfig(program, params) {
  console.log("🔧 Updating protocol configuration...");

  try {
    // Derive protocol config PDA
    const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId
    );

    console.log(`Protocol Config PDA: ${protocolConfigPDA.toString()}`);
    console.log("");

    // Read current config for comparison
    const currentConfig = await program.account.protocolConfig.fetch(
      protocolConfigPDA
    );

    console.log("📋 Current Configuration:");
    console.log(`   Authority: ${currentConfig.authority.toString()}`);
    console.log(
      `   Underlying Token: ${currentConfig.underlyingToken.toString()}`
    );
    console.log(
      `   Base Price per Second (scaled): ${currentConfig.basePricePerSecondScaled.toLocaleString()}`
    );
    console.log(
      `   Frequency Coefficient: ${formatBasisPoints(
        currentConfig.frequencyCoefficient
      )}`
    );
    console.log(
      `   Signers Coefficient: ${formatBasisPoints(
        currentConfig.signersCoefficient
      )}`
    );
    console.log(
      `   Reward Percentage: ${formatBasisPoints(
        currentConfig.rewardPercentage
      )}`
    );
    console.log(
      `   Security Deposit: ${currentConfig.nodeSecurityDeposit.toString()}`
    );
    console.log("");

    // Prepare update parameters
    const updateParams = {
      basePricePerSecondScaled: params.basePricePerSecondScaled
        ? parseTokenAmount(params.basePricePerSecondScaled, 6)
        : null, // Scaled by 1M
      frequencyCoefficient: params.frequencyCoefficient
        ? parseBasisPoints(params.frequencyCoefficient)
        : null,
      signersCoefficient: params.signersCoefficient
        ? parseBasisPoints(params.signersCoefficient)
        : null,
      rewardPercentage: params.rewardPercentage
        ? parseBasisPoints(params.rewardPercentage)
        : null,
      nodeSecurityDeposit: params.securityDeposit
        ? parseTokenAmount(params.securityDeposit, 6)
        : null,
    };

    if (Object.keys(updateParams).length === 0) {
      console.log(
        "⚠️  No parameters to update. Please provide at least one parameter to update."
      );
      return;
    }

    console.log("🔄 Updating Parameters:");
    Object.entries(updateParams).forEach(([key, value]) => {
      if (value === null) {
        return;
      }
      if (key === "underlyingToken") {
        console.log(`   ${key}: ${value.toString()}`);
      } else if (key === "basePricePerSecondScaled") {
        console.log(
          `   ${key}: ${value.toLocaleString()} (${formatTokenAmount(
            value,
            6
          )} tokens)`
        );
      } else {
        console.log(`   ${key}: ${formatBasisPoints(value)}`);
      }
    });
    console.log("");

    // Execute the update
    const tx = await program.methods
      .updateProtocolConfig(updateParams)
      .accounts({
        protocolConfig: protocolConfigPDA,
        authority: program.provider.wallet.publicKey,
      })
      .rpc();

    console.log("✅ Protocol configuration updated successfully!");
    console.log(`Transaction signature: ${tx}`);
    console.log("");

    // Read updated config
    const updatedConfig = await program.account.protocolConfig.fetch(
      protocolConfigPDA
    );

    console.log("📋 Updated Configuration:");
    console.log(`   Authority: ${updatedConfig.authority.toString()}`);
    console.log(
      `   Underlying Token: ${updatedConfig.underlyingToken.toString()}`
    );
    console.log(
      `   Base Price per Second (scaled): ${updatedConfig.basePricePerSecondScaled.toLocaleString()}`
    );
    console.log(
      `   Frequency Coefficient: ${formatBasisPoints(
        updatedConfig.frequencyCoefficient
      )}`
    );
    console.log(
      `   Signers Coefficient: ${formatBasisPoints(
        updatedConfig.signersCoefficient
      )}`
    );
    console.log(
      `   Reward Percentage: ${formatBasisPoints(
        updatedConfig.rewardPercentage
      )}`
    );
    console.log("");
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(
        "❌ Protocol configuration not found. The protocol may not be initialized."
      );
      console.log(
        "💡 Initialize the protocol with: node scripts/initialize.js <TOKEN_MINT>"
      );
    } else {
      console.error("❌ Error updating protocol configuration:", error.message);
    }
    throw error;
  }
}

function showUsage() {
  console.log("📖 Molpha Protocol Configuration Updater");
  console.log("");
  console.log("Usage:");
  console.log("  node update-protocol-config.js [options]");
  console.log("");
  console.log("Options:");
  console.log(
    "  --security-deposit <AMOUNT>     Update security deposit (in tokens, e.g., '1000')"
  );
  console.log(
    "  --base-price <AMOUNT>            Update base price per second (in tokens, e.g., '0.001')"
  );
  console.log(
    "  --frequency-coefficient <BP>     Update frequency coefficient (basis points, e.g., '500' or '5%')"
  );
  console.log(
    "  --signers-coefficient <BP>       Update signers coefficient (basis points, e.g., '1000' or '10%')"
  );
  console.log(
    "  --reward-percentage <BP>         Update reward percentage (basis points, e.g., '2000' or '20%')"
  );
  console.log("  --help, -h                       Show this help");
  console.log("");
  console.log("Examples:");
  console.log("  # Update base price to 0.001 tokens per second");
  console.log("  node update-protocol-config.js --base-price 0.001");
  console.log("");
  console.log("  # Update frequency coefficient to 5%");
  console.log("  node update-protocol-config.js --frequency-coefficient 5%");
  console.log("");
  console.log("  # Update multiple parameters");
  console.log(
    "  node update-protocol-config.js --base-price 0.002 --frequency-coefficient 10% --signers-coefficient 15%"
  );
  console.log("");
  console.log("  # Update underlying token");
  console.log(
    "  node update-protocol-config.js --underlying-token EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
  );
  console.log("");
}

async function main() {
  // Parse command line arguments
  const args = process.argv.slice(2);

  if (args.includes("--help") || args.includes("-h") || args.length === 0) {
    showUsage();
    process.exit(0);
  }

  const params = {};

  for (let i = 0; i < args.length; i += 2) {
    const key = args[i];
    const value = args[i + 1];

    switch (key) {
      case "--security-deposit":
        params.securityDeposit = value;
        break;
      case "--base-price":
        params.basePricePerSecondScaled = value;
        break;
      case "--frequency-coefficient":
        params.frequencyCoefficient = value;
        break;
      case "--signers-coefficient":
        params.signersCoefficient = value;
        break;
      case "--reward-percentage":
        params.rewardPercentage = value;
        break;
      default:
        console.error(`❌ Unknown option: ${key}`);
        showUsage();
        process.exit(1);
    }
  }

  // Setup environment
  setupEnvironment();

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("Authority:", provider.wallet.publicKey.toString());
    console.log("");

    await updateProtocolConfig(program, params);
  } catch (error) {
    console.error("❌ Error:", error.message);
    if (error.logs) {
      console.error("Transaction logs:", error.logs);
    }
    process.exit(1);
  }
}

main().catch(console.error);
