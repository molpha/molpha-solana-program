#!/usr/bin/env node

/**
 * Generate EVM Keypair Script
 *
 * This script generates a new secp256k1 keypair for EVM compatibility.
 * It outputs:
 * - Private key (32 bytes)
 * - Public key - uncompressed (65 bytes)
 * - Public key - compressed (33 bytes) - for node registration
 * - Ethereum address (20 bytes)
 *
 * Usage: node scripts/generate-evm-keypair.js [--save <filepath>]
 */

const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const fs = require("fs");
const path = require("path");

function generateKeypair() {
  // Generate random private key
  const privateKey = secp256k1.utils.randomPrivateKey();

  // Derive public key from private key
  const publicKeyPoint = secp256k1.ProjectivePoint.fromPrivateKey(privateKey);

  // Get uncompressed public key (65 bytes: 0x04 + x + y)
  const uncompressedPubkey = publicKeyPoint.toRawBytes(false);

  // Get compressed public key (33 bytes: 0x02/0x03 + x)
  const compressedPubkey = publicKeyPoint.toRawBytes(true);

  // Calculate Ethereum address
  // Address = last 20 bytes of keccak256(uncompressed pubkey without 0x04 prefix)
  const pubkeyWithoutPrefix = uncompressedPubkey.slice(1); // Remove 0x04 prefix
  const hash = keccak_256(pubkeyWithoutPrefix);
  const address = hash.slice(-20); // Take last 20 bytes

  return {
    privateKey: Buffer.from(privateKey),
    publicKeyUncompressed: Buffer.from(uncompressedPubkey),
    publicKeyCompressed: Buffer.from(compressedPubkey),
    address: Buffer.from(address),
  };
}

function saveToFile(keypair, filepath) {
  const data = {
    privateKey: keypair.privateKey.toString("hex"),
    publicKey: {
      uncompressed: keypair.publicKeyUncompressed.toString("hex"),
      compressed: keypair.publicKeyCompressed.toString("hex"),
    },
    address: "0x" + keypair.address.toString("hex"),
    generatedAt: new Date().toISOString(),
  };

  const dir = path.dirname(filepath);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  fs.writeFileSync(filepath, JSON.stringify(data, null, 2), "utf8");

  // Set restrictive permissions (owner read/write only)
  try {
    fs.chmodSync(filepath, 0o600);
  } catch (e) {
    // Ignore on platforms that don't support chmod
  }
}

function displayKeypair(keypair) {
  // Extract x and y coordinates from uncompressed public key
  // Uncompressed format: 0x04 + x (32 bytes) + y (32 bytes)
  const x = keypair.publicKeyUncompressed.slice(1, 33);
  const y = keypair.publicKeyUncompressed.slice(33, 65);

  // Convert to BigInt for display as uint256
  const xBigInt = BigInt("0x" + x.toString("hex"));
  const yBigInt = BigInt("0x" + y.toString("hex"));

  // Verify compression logic
  const isYEven = yBigInt % 2n === 0n;
  const expectedPrefix = isYEven ? "02" : "03";
  const actualPrefix = keypair.publicKeyCompressed.slice(0, 1).toString("hex");

  console.log("🔐 New EVM Keypair Generated");
  console.log("=".repeat(80));
  console.log("");

  console.log("🔑 Private Key (Keep this secret!):");
  console.log("  Hex:", keypair.privateKey.toString("hex"));
  console.log("  With 0x:", "0x" + keypair.privateKey.toString("hex"));
  console.log("  Length:", keypair.privateKey.length, "bytes");
  console.log("");

  console.log("🔓 Public Key (Uncompressed):");
  console.log("  Hex:", keypair.publicKeyUncompressed.toString("hex"));
  console.log(
    "  With 0x:",
    "0x" + keypair.publicKeyUncompressed.toString("hex")
  );
  console.log("  Length:", keypair.publicKeyUncompressed.length, "bytes");
  console.log("  Format: 0x04 + x-coordinate + y-coordinate");
  console.log("");

  console.log("📐 Public Key (Affine Coordinates):");
  console.log(
    "  /// @dev Point encapsulates a secp256k1 point in Affine coordinates."
  );
  console.log("  struct Point {");
  console.log("      uint x;");
  console.log("      uint y;");
  console.log("  }");
  console.log("");
  console.log("  Point({");
  console.log("      x:", xBigInt.toString() + ",");
  console.log("      y:", yBigInt.toString());
  console.log("  });");
  console.log("");
  console.log("  X (hex):", "0x" + x.toString("hex"));
  console.log("  Y (hex):", "0x" + y.toString("hex"));
  console.log("  Y is:", isYEven ? "even" : "odd");
  console.log("");

  console.log("📦 Public Key (Compressed) - Use this for node registration:");
  console.log("  Hex:", keypair.publicKeyCompressed.toString("hex"));
  console.log("  With 0x:", "0x" + keypair.publicKeyCompressed.toString("hex"));
  console.log("  Length:", keypair.publicKeyCompressed.length, "bytes");
  console.log("  Format: 0x" + expectedPrefix + " + x-coordinate");
  console.log(
    "  Prefix verification:",
    actualPrefix === expectedPrefix ? "✅ correct" : "❌ mismatch"
  );
  console.log("");

  console.log("📮 Ethereum Address:");
  console.log("  Address:", "0x" + keypair.address.toString("hex"));
  console.log("  Length:", keypair.address.length, "bytes");
  console.log(
    "  Checksum:",
    toChecksumAddress("0x" + keypair.address.toString("hex"))
  );
  console.log("");

  console.log("=".repeat(80));
}

function compressedPubkeyToAddress(compressedPubkey) {
  // Accept hex string (with or without 0x) or Buffer
  let pubkeyBytes;
  if (typeof compressedPubkey === "string") {
    const hex = compressedPubkey.replace("0x", "");
    pubkeyBytes = Buffer.from(hex, "hex");
  } else {
    pubkeyBytes = compressedPubkey;
  }

  // Validate compressed public key format
  if (pubkeyBytes.length !== 33) {
    throw new Error(
      `Invalid compressed public key length: ${pubkeyBytes.length} bytes (expected 33)`
    );
  }

  const prefix = pubkeyBytes[0];
  if (prefix !== 0x02 && prefix !== 0x03) {
    throw new Error(
      `Invalid compressed public key prefix: 0x${prefix.toString(
        16
      )} (expected 0x02 or 0x03)`
    );
  }

  // Decompress the public key
  const publicKeyPoint = secp256k1.ProjectivePoint.fromHex(pubkeyBytes);
  const uncompressedPubkey = publicKeyPoint.toRawBytes(false);

  // Calculate Ethereum address
  // Address = last 20 bytes of keccak256(uncompressed pubkey without 0x04 prefix)
  const pubkeyWithoutPrefix = uncompressedPubkey.slice(1); // Remove 0x04 prefix
  const hash = keccak_256(pubkeyWithoutPrefix);
  const address = hash.slice(-20); // Take last 20 bytes

  return Buffer.from(address);
}

function toChecksumAddress(address) {
  // Simple checksum implementation (EIP-55)
  const addr = address.toLowerCase().replace("0x", "");
  const hash = Buffer.from(keccak_256(Buffer.from(addr, "utf-8"))).toString(
    "hex"
  );
  let checksumAddr = "0x";

  for (let i = 0; i < addr.length; i++) {
    if (parseInt(hash[i], 16) >= 8) {
      checksumAddr += addr[i].toUpperCase();
    } else {
      checksumAddr += addr[i];
    }
  }

  return checksumAddr;
}

function main() {
  let savePath = null;

  // Parse command line arguments
  for (let i = 2; i < process.argv.length; i++) {
    if (process.argv[i] === "--save" || process.argv[i] === "-s") {
      savePath = process.argv[i + 1];
      i++; // Skip next argument
    } else if (process.argv[i] === "--help" || process.argv[i] === "-h") {
      console.log("Generate EVM Keypair");
      console.log("");
      console.log("Usage: node scripts/generate-evm-keypair.js [options]");
      console.log("");
      console.log("Options:");
      console.log("  --save, -s <filepath>    Save keypair to JSON file");
      console.log("  --help, -h               Show this help message");
      console.log("");
      console.log("Examples:");
      console.log("  node scripts/generate-evm-keypair.js");
      console.log(
        "  node scripts/generate-evm-keypair.js --save ./keys/evm-keypair.json"
      );
      console.log("");
      console.log("Output:");
      console.log("  - Private key (32 bytes hex)");
      console.log("  - Public key uncompressed (65 bytes hex)");
      console.log(
        "  - Public key compressed (33 bytes hex) - for node registration"
      );
      console.log("  - Ethereum address (20 bytes hex with checksum)");
      console.log("");
      process.exit(0);
    }
  }

  try {
    // Generate keypair
    const keypair = generateKeypair();

    // Display keypair
    displayKeypair(keypair);

    // Verify address derivation from compressed pubkey
    const derivedAddress = compressedPubkeyToAddress(
      keypair.publicKeyCompressed
    );
    const addressesMatch = derivedAddress.equals(keypair.address);
    console.log("✅ Verification:");
    console.log(
      "  Derived address from compressed pubkey:",
      "0x" + derivedAddress.toString("hex")
    );
    console.log("  Address match:", addressesMatch ? "✅ PASS" : "❌ FAIL");
    console.log("");

    // Save if requested
    if (savePath) {
      saveToFile(keypair, savePath);
      console.log("💾 Keypair saved to:", path.resolve(savePath));
      console.log("   File permissions: 600 (owner read/write only)");
      console.log("");
    }

    // Usage instructions
    console.log("💡 Usage Instructions:");
    console.log("");
    console.log("1. Register node with compressed public key:");
    console.log(
      `   yarn molpha registry register --secp-pubkey 0x${keypair.publicKeyCompressed.toString(
        "hex"
      )} --registration-salt <32_BYTE_HEX>`
    );
    console.log("");
    console.log("2. Compress an existing uncompressed public key:");
    console.log(
      `   node scripts/compress-evm-pubkey.js 0x${keypair.publicKeyUncompressed.toString(
        "hex"
      )}`
    );
    console.log("");

    if (!savePath) {
      console.log(
        "⚠️  WARNING: Keypair not saved! Save it now if you want to keep it:"
      );
      console.log(
        `   node scripts/generate-evm-keypair.js --save ./keys/evm-keypair.json`
      );
      console.log("");
    }
  } catch (error) {
    console.error("❌ Error:", error.message);
    process.exit(1);
  }
}

// Run main if executed directly
if (require.main === module) {
  main();
}

// Export functions for use in other scripts
module.exports = {
  generateKeypair,
  compressedPubkeyToAddress,
  toChecksumAddress,
};
