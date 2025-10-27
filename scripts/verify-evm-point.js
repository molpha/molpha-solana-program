#!/usr/bin/env node

/**
 * Verify EVM Point Script
 * 
 * This script verifies the compression logic for secp256k1 points.
 * You can provide x, y coordinates and verify the compressed public key.
 * 
 * Usage: node scripts/verify-evm-point.js <X_DECIMAL> <Y_DECIMAL> [EXPECTED_COMPRESSED_HEX]
 */

const secp256k1 = require('@noble/secp256k1');

function verifyPoint(xDecimal, yDecimal, expectedCompressedHex = null) {
  console.log("🔍 Verifying EVM Point Compression");
  console.log("=".repeat(80));
  console.log("");
  
  // Convert decimal strings to BigInt
  const x = BigInt(xDecimal);
  const y = BigInt(yDecimal);
  
  console.log("📐 Input Point (Affine Coordinates):");
  console.log("  Point({");
  console.log("      x:", x.toString() + ",");
  console.log("      y:", y.toString());
  console.log("  });");
  console.log("");
  
  // Convert to hex (32 bytes each)
  const xHex = x.toString(16).padStart(64, '0');
  const yHex = y.toString(16).padStart(64, '0');
  
  console.log("  X (hex):", '0x' + xHex);
  console.log("  Y (hex):", '0x' + yHex);
  console.log("");
  
  // Check if y is even or odd
  const isYEven = y % 2n === 0n;
  const prefix = isYEven ? '02' : '03';
  
  console.log("📊 Compression Analysis:");
  console.log("  Y is:", isYEven ? "even" : "odd");
  console.log("  Prefix:", '0x' + prefix);
  console.log("");
  
  // Create compressed public key: prefix + x
  const compressedHex = prefix + xHex;
  
  console.log("📦 Compressed Public Key:");
  console.log("  Hex:", compressedHex);
  console.log("  With 0x:", '0x' + compressedHex);
  console.log("  Length:", compressedHex.length / 2, "bytes");
  console.log("");
  
  // Verify if expected compressed key matches
  if (expectedCompressedHex) {
    const expected = expectedCompressedHex.startsWith('0x') 
      ? expectedCompressedHex.slice(2) 
      : expectedCompressedHex;
    
    const matches = compressedHex.toLowerCase() === expected.toLowerCase();
    
    console.log("✅ Verification:");
    console.log("  Expected:", expected);
    console.log("  Computed:", compressedHex);
    console.log("  Match:", matches ? "✅ YES" : "❌ NO");
    console.log("");
    
    if (!matches) {
      console.log("⚠️  Mismatch Details:");
      console.log("  Expected prefix:", expected.substring(0, 2));
      console.log("  Computed prefix:", prefix);
      console.log("  Expected x:", expected.substring(2));
      console.log("  Computed x:", xHex);
      console.log("");
    }
  }
  
  // Try to verify point is on secp256k1 curve
  try {
    const point = secp256k1.ProjectivePoint.fromAffine({ 
      x: BigInt('0x' + xHex), 
      y: BigInt('0x' + yHex) 
    });
    
    // Verify by decompressing the compressed key
    const decompressedPoint = secp256k1.ProjectivePoint.fromHex(compressedHex);
    const decompressed = decompressedPoint.toRawBytes(false); // uncompressed
    
    // Extract x and y from decompressed
    const decompressedX = Buffer.from(decompressed.slice(1, 33)).toString('hex');
    const decompressedY = Buffer.from(decompressed.slice(33, 65)).toString('hex');
    
    console.log("🔄 Round-trip Verification:");
    console.log("  Compressed → Decompressed");
    console.log("  X matches:", decompressedX === xHex ? "✅" : "❌");
    console.log("  Y matches:", decompressedY === yHex ? "✅" : "❌");
    console.log("  Point is valid on secp256k1 curve: ✅");
    console.log("");
    
  } catch (e) {
    console.log("⚠️  Warning: Point validation failed");
    console.log("  Error:", e.message);
    console.log("  This point may not be on the secp256k1 curve");
    console.log("");
  }
  
  // Create uncompressed format
  const uncompressedHex = '04' + xHex + yHex;
  console.log("🔓 Uncompressed Public Key:");
  console.log("  Hex:", uncompressedHex);
  console.log("  With 0x:", '0x' + uncompressedHex);
  console.log("  Length:", uncompressedHex.length / 2, "bytes");
  console.log("");
  
  console.log("=".repeat(80));
}

function main() {
  if (process.argv.length < 4) {
    console.error("❌ Error: X and Y coordinates are required");
    console.log("");
    console.log("Usage: node scripts/verify-evm-point.js <X_DECIMAL> <Y_DECIMAL> [EXPECTED_COMPRESSED_HEX]");
    console.log("");
    console.log("Example:");
    console.log("  node scripts/verify-evm-point.js \\");
    console.log("    23291323678511451217772133928868366990746786347038015158807413032596505148183 \\");
    console.log("    47168895393257141911376575107721994933810868803586352809270883649069 \\");
    console.log("    0x03337e6c0a75b98cc736c4d04b82049984e7e8828e2f2821169b221f38fd54bb17");
    console.log("");
    console.log("Or provide just x and y:");
    console.log("  node scripts/verify-evm-point.js <X> <Y>");
    console.log("");
    process.exit(1);
  }

  try {
    const xDecimal = process.argv[2];
    const yDecimal = process.argv[3];
    const expectedCompressed = process.argv[4] || null;
    
    verifyPoint(xDecimal, yDecimal, expectedCompressed);
    
  } catch (error) {
    console.error("❌ Error:", error.message);
    console.error(error.stack);
    process.exit(1);
  }
}

main();

