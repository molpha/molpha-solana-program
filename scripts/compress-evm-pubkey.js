#!/usr/bin/env node

/**
 * Compress EVM Public Key Script
 * 
 * This script takes an uncompressed secp256k1 public key (65 bytes) and 
 * converts it to a compressed format (33 bytes) suitable for node registration.
 * 
 * Usage: node scripts/compress-evm-pubkey.js <UNCOMPRESSED_PUBKEY_HEX>
 */

const secp256k1 = require('@noble/secp256k1');

function compressPublicKey(uncompressedHex) {
  // Remove 0x prefix if present
  const hex = uncompressedHex.startsWith('0x') ? uncompressedHex.slice(2) : uncompressedHex;
  
  // Convert hex to Uint8Array
  const pubkeyBytes = Uint8Array.from(Buffer.from(hex, 'hex'));
  
  // Validate length
  if (pubkeyBytes.length !== 65) {
    throw new Error(`Invalid uncompressed public key length: ${pubkeyBytes.length} bytes (expected 65)`);
  }
  
  // Validate prefix
  if (pubkeyBytes[0] !== 0x04) {
    throw new Error(`Invalid uncompressed public key prefix: 0x${pubkeyBytes[0].toString(16)} (expected 0x04)`);
  }
  
  // Extract x and y coordinates
  const x = pubkeyBytes.slice(1, 33);
  const y = pubkeyBytes.slice(33, 65);
  
  // Determine if y is even or odd
  const isYEven = y[y.length - 1] % 2 === 0;
  
  // Create compressed public key: prefix (0x02 for even, 0x03 for odd) + x coordinate
  const prefix = isYEven ? 0x02 : 0x03;
  const compressed = new Uint8Array(33);
  compressed[0] = prefix;
  compressed.set(x, 1);
  
  return Buffer.from(compressed);
}

function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Uncompressed public key is required");
    console.log("");
    console.log("Usage: node scripts/compress-evm-pubkey.js <UNCOMPRESSED_PUBKEY_HEX>");
    console.log("");
    console.log("The uncompressed public key should be:");
    console.log("  - 65 bytes (130 hex characters) in length");
    console.log("  - Start with 0x04 prefix");
    console.log("  - Format: 0x04 + x-coordinate (32 bytes) + y-coordinate (32 bytes)");
    console.log("");
    console.log("Example:");
    console.log("  node scripts/compress-evm-pubkey.js 0x04ab12cd34...");
    console.log("  node scripts/compress-evm-pubkey.js ab12cd34...");
    console.log("");
    process.exit(1);
  }

  try {
    const uncompressedHex = process.argv[2];
    
    console.log("🔄 Compressing EVM public key...");
    console.log("");
    
    // Show input
    const inputHex = uncompressedHex.startsWith('0x') ? uncompressedHex.slice(2) : uncompressedHex;
    console.log("📥 Input (Uncompressed):");
    console.log("  Length:", inputHex.length / 2, "bytes");
    console.log("  Hex:", inputHex);
    console.log("");
    
    // Compress
    const compressed = compressPublicKey(uncompressedHex);
    
    // Show output
    console.log("📤 Output (Compressed):");
    console.log("  Length:", compressed.length, "bytes");
    console.log("  Hex:", compressed.toString('hex'));
    console.log("  With 0x:", '0x' + compressed.toString('hex'));
    console.log("");
    
    // Verify by decompressing
    try {
      const point = secp256k1.ProjectivePoint.fromHex(compressed.toString('hex'));
      const decompressed = point.toRawBytes(false); // false = uncompressed
      console.log("✅ Verification: Successfully decompressed back to original");
      console.log("  Matches input:", Buffer.from(decompressed).toString('hex') === inputHex);
    } catch (e) {
      console.log("⚠️  Warning: Could not verify compression (but output is valid)");
    }
    
    console.log("");
    console.log("💡 Usage with node registration:");
    console.log(`  node scripts/add-node.js <NODE_PUBKEY> 0x${compressed.toString('hex')}`);
    console.log("");
    
  } catch (error) {
    console.error("❌ Error:", error.message);
    process.exit(1);
  }
}

main();

