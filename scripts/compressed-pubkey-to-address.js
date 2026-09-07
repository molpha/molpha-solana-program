#!/usr/bin/env node

/**
 * Compressed EVM Public Key to Address Converter
 *
 * This script converts a compressed secp256k1 public key to an Ethereum address.
 *
 * Usage: node scripts/compressed-pubkey-to-address.js <COMPRESSED_PUBKEY>
 *
 * Example:
 *   node scripts/compressed-pubkey-to-address.js 0x03f0e28e2424c3100a9d0ecc3ffe2d3319a1e208c3678c6d0e29cad8d1e952d3c9
 */

const {
  compressedPubkeyToAddress,
  toChecksumAddress,
} = require("./generate-evm-keypair.js");

function main() {
  // Show help if requested or no arguments
  if (
    process.argv.length < 3 ||
    process.argv.includes("--help") ||
    process.argv.includes("-h")
  ) {
    console.log("Compressed EVM Public Key to Address Converter");
    console.log("");
    console.log(
      "Usage: node scripts/compressed-pubkey-to-address.js <COMPRESSED_PUBKEY>",
    );
    console.log("");
    console.log("Arguments:");
    console.log(
      "  COMPRESSED_PUBKEY    33-byte compressed public key (hex, with or without 0x prefix)",
    );
    console.log("");
    console.log("Options:");
    console.log("  --help, -h          Show this help message");
    console.log("");
    console.log("Examples:");
    console.log(
      "  node scripts/compressed-pubkey-to-address.js 0x03f0e28e2424c3100a9d0ecc3ffe2d3319a1e208c3678c6d0e29cad8d1e952d3c9",
    );
    console.log(
      "  node scripts/compressed-pubkey-to-address.js 03f0e28e2424c3100a9d0ecc3ffe2d3319a1e208c3678c6d0e29cad8d1e952d3c9",
    );
    console.log("");
    process.exit(0);
  }

  const compressedPubkey = process.argv[2];

  try {
    console.log("🔍 Converting Compressed Public Key to Address");
    console.log("=".repeat(80));
    console.log("");

    console.log("📦 Input (Compressed Public Key):");
    console.log("  ", compressedPubkey);
    console.log("");

    // Convert to address
    const address = compressedPubkeyToAddress(compressedPubkey);
    const addressHex = "0x" + address.toString("hex");
    const checksumAddress = toChecksumAddress(addressHex);

    console.log("📮 Output (Ethereum Address):");
    console.log("  Hex:      ", addressHex);
    console.log("  Checksum: ", checksumAddress);
    console.log("  Length:   ", address.length, "bytes");
    console.log("");

    console.log("=".repeat(80));
    console.log("✅ Conversion successful!");
  } catch (error) {
    console.error("");
    console.error("❌ Error:", error.message);
    console.error("");
    console.error("Expected format:");
    console.error("  - 33 bytes (66 hex characters)");
    console.error("  - Starting with 0x02 or 0x03");
    console.error("  - With or without 0x prefix");
    console.error("");
    console.error("Example valid input:");
    console.error(
      "  0x03f0e28e2424c3100a9d0ecc3ffe2d3319a1e208c3678c6d0e29cad8d1e952d3c9",
    );
    console.error("");
    process.exit(1);
  }
}

main();
