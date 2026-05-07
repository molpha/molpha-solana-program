/**
 * Example: Using Molpha with Standard Oracle Interfaces
 *
 * This example demonstrates how to:
 * 1. Publish price data to Molpha feeds
 * 2. Read data using Pyth-compatible interface
 * 3. Read data using Switchboard-compatible interface
 * 4. Read data using generic interface
 */

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import {
  MolphaOracleClient,
  OracleEncoder,
  PriceStatus,
} from "./oracle-client";

async function main() {
  // Setup
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Molpha as Program;

  const client = new MolphaOracleClient(program, provider.connection);

  // Example feed address (replace with your actual feed)
  const feedAddress = new PublicKey("YOUR_FEED_ADDRESS_HERE");

  console.log("=".repeat(60));
  console.log("Molpha Oracle Interface Examples");
  console.log("=".repeat(60));

  // ============================================================================
  // Example 1: Reading with Pyth Interface
  // ============================================================================
  console.log("\n📊 Example 1: Pyth-Compatible Interface\n");

  try {
    const pythPrice = await client.getPythPrice(feedAddress);

    console.log("Raw Pyth Data:");
    console.log(`  Price:        ${pythPrice.price.toString()}`);
    console.log(`  Confidence:   ${pythPrice.conf.toString()}`);
    console.log(`  Exponent:     ${pythPrice.expo}`);
    console.log(
      `  Publish Time: ${new Date(
        pythPrice.publishTime.toNumber() * 1000
      ).toISOString()}`
    );

    // Calculate actual decimal price
    const actualPrice = Number(pythPrice.price) * Math.pow(10, pythPrice.expo);
    const actualConf = Number(pythPrice.conf) * Math.pow(10, pythPrice.expo);

    console.log("\nDecimal Values:");
    console.log(`  Price:      $${actualPrice.toFixed(2)}`);
    console.log(`  Confidence: ±$${actualConf.toFixed(2)}`);
    console.log(
      `  Range:      $${(actualPrice - actualConf).toFixed(2)} - $${(
        actualPrice + actualConf
      ).toFixed(2)}`
    );
  } catch (error) {
    console.error("Error reading Pyth price:", error.message);
  }

  // ============================================================================
  // Example 2: Reading with Staleness Check
  // ============================================================================
  console.log("\n⏰ Example 2: Price with Staleness Check\n");

  try {
    // Only accept prices updated within the last 60 seconds
    const freshPrice = await client.getPythPriceNoOlderThan(feedAddress, 60);
    console.log("✅ Price is fresh (< 60 seconds old)");
    console.log(
      `   Updated: ${new Date(
        freshPrice.publishTime.toNumber() * 1000
      ).toISOString()}`
    );
  } catch (error) {
    console.log("❌ Price is too old:", error.message);
  }

  // ============================================================================
  // Example 3: Reading with Switchboard Interface
  // ============================================================================
  console.log("\n📈 Example 3: Switchboard-Compatible Interface\n");

  try {
    const sbResult = await client.getSwitchboardResult(feedAddress, 8);

    console.log("Switchboard Data:");
    console.log(`  Result:       $${sbResult.result.toFixed(2)}`);
    console.log(`  Std Dev:      $${sbResult.stdDev.toFixed(2)}`);
    console.log(`  Min Response: $${sbResult.minResponse.toFixed(2)}`);
    console.log(`  Max Response: $${sbResult.maxResponse.toFixed(2)}`);
    console.log(`  Success:      ${sbResult.numSuccess}`);
    console.log(`  Errors:       ${sbResult.numError}`);
  } catch (error) {
    console.error("Error reading Switchboard result:", error.message);
  }

  // ============================================================================
  // Example 4: Generic Price Interface (Easiest)
  // ============================================================================
  console.log("\n🎯 Example 4: Generic Price Interface (Recommended)\n");

  try {
    const price = await client.getPrice(feedAddress);

    console.log("Simple Price Data:");
    console.log(`  Value:       $${price.value.toFixed(2)}`);
    console.log(`  Confidence:  ±$${price.confidence?.toFixed(2)}`);
    console.log(`  Status:      ${price.status.toUpperCase()}`);
    console.log(
      `  Timestamp:   ${new Date(price.timestamp * 1000).toISOString()}`
    );

    // Check status
    if (price.status === PriceStatus.Trading) {
      console.log("\n✅ Feed is active and trading");
    } else if (price.status === PriceStatus.Halted) {
      console.log("\n⚠️  Feed subscription has expired");
    } else {
      console.log("\n⚠️  Feed data is stale");
    }
  } catch (error) {
    console.error("Error reading price:", error.message);
  }

  // ============================================================================
  // Example 5: Reading Price History
  // ============================================================================
  console.log("\n📜 Example 5: Price History\n");

  try {
    const history = await client.getPriceHistory(feedAddress);

    console.log(`Last ${history.length} updates:`);
    history.slice(-5).forEach((p, i) => {
      const date = new Date(p.timestamp * 1000);
      console.log(
        `  ${i + 1}. $${p.value.toFixed(2)} at ${date.toISOString()}`
      );
    });
  } catch (error) {
    console.error("Error reading history:", error.message);
  }

  // ============================================================================
  // Example 6: Checking Subscription Status
  // ============================================================================
  console.log("\n💳 Example 6: Subscription Status\n");

  try {
    const isActive = await client.isSubscriptionActive(feedAddress);
    console.log(`Subscription Active: ${isActive ? "✅ Yes" : "❌ No"}`);

    if (!isActive) {
      console.log(
        "⚠️  Feed needs subscription extension to continue receiving updates"
      );
    }
  } catch (error) {
    console.error("Error checking subscription:", error.message);
  }

  // ============================================================================
  // Example 7: Encoding Price Data for Publishing
  // ============================================================================
  console.log("\n📝 Example 7: Encoding Price Data (for Oracle Nodes)\n");

  // Example: BTC/USD = $50,000.00 with $10 confidence
  const btcPrice = 50_000n * 100_000_000n; // $50,000 with 8 decimals
  const btcConf = 10n * 100_000_000n; // $10 confidence
  const btcExpo = -8; // 8 decimal places

  const encodedPrice = OracleEncoder.encodePythPrice(
    btcPrice,
    btcConf,
    btcExpo
  );

  console.log("Price Encoding Example (BTC/USD):");
  console.log(`  Input:  $50,000.00 ± $10.00`);
  console.log(`  Encoded: 0x${encodedPrice.toString("hex")}`);
  console.log(`  Ready to publish via publish_answer instruction`);

  // Example: ETH/USD aggregator data
  const ethValue = 3_000n * 100_000_000n; // $3,000
  const ethStdDev = 5n * 100_000_000n; // $5 std dev
  const ethMin = 2_995n * 100_000_000n; // $2,995
  const ethMax = 3_005n * 100_000_000n; // $3,005

  const encodedAggregator = OracleEncoder.encodeSwitchboardResult(
    ethValue,
    ethStdDev,
    ethMin,
    ethMax
  );

  console.log("\nAggregator Encoding Example (ETH/USD):");
  console.log(`  Value:  $3,000.00`);
  console.log(`  StdDev: $5.00`);
  console.log(`  Range:  $2,995.00 - $3,005.00`);
  console.log(`  Encoded: 0x${encodedAggregator.toString("hex")}`);

  console.log("\n" + "=".repeat(60));
  console.log("Examples Complete!");
  console.log("=".repeat(60) + "\n");
}

// Run the examples
main()
  .then(() => {
    console.log("✅ All examples executed successfully");
    process.exit(0);
  })
  .catch((error) => {
    console.error("❌ Error running examples:", error);
    process.exit(1);
  });

/**
 * Usage Instructions:
 *
 * 1. Set up your environment:
 *    export ANCHOR_PROVIDER_URL="https://api.devnet.solana.com"
 *    export ANCHOR_WALLET="~/.config/solana/id.json"
 *
 * 2. Replace 'YOUR_FEED_ADDRESS_HERE' with an actual feed address
 *
 * 3. Run the example:
 *    npx ts-node scripts/example-oracle-usage.ts
 *
 * Integration in Your DApp:
 *
 * ```typescript
 * import { MolphaOracleClient } from './scripts/oracle-client';
 *
 * const client = new MolphaOracleClient(program, connection);
 *
 * // Simple integration
 * const price = await client.getPrice(feedAddress);
 * if (price.status === 'trading') {
 *   console.log(`Current BTC/USD: $${price.value}`);
 * }
 *
 * // Pyth-style integration
 * const pythPrice = await client.getPythPriceNoOlderThan(feedAddress, 60);
 * const actualPrice = Number(pythPrice.price) * Math.pow(10, pythPrice.expo);
 * ```
 */
