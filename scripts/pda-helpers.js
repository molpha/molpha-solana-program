const crypto = require('crypto');

/**
 * Hash a name string to a 32-byte seed for PDA derivation
 * This matches the Rust to_seed() function in utils/seeds.rs
 * @param {string} name - The name to hash
 * @returns {Buffer} 32-byte hash as Buffer
 */
function toSeed(name) {
  return crypto.createHash('sha256').update(name, 'utf-8').digest();
}

/**
 * Derive a feed PDA
 * @param {import('@solana/web3.js').PublicKey} programId - The program ID
 * @param {import('@solana/web3.js').PublicKey} authority - The authority public key
 * @param {string} name - The feed name
 * @param {number} feedType - The feed type (0 = Public, 1 = Personal)
 * @returns {[import('@solana/web3.js').PublicKey, number]} The PDA and bump seed
 */
function getFeedPda(programId, authority, name, feedType) {
  const anchor = require('@coral-xyz/anchor');
  
  // Convert feed type to buffer (single byte)
  const feedTypeBuffer = Buffer.from([feedType]);
  
  return anchor.web3.PublicKey.findProgramAddressSync(
    [
      Buffer.from('feed'),
      authority.toBuffer(),
      toSeed(name),
      feedTypeBuffer,
    ],
    programId
  );
}

/**
 * Derive a data source PDA
 * @param {import('@solana/web3.js').PublicKey} programId - The program ID
 * @param {import('@solana/web3.js').PublicKey} authority - The authority public key
 * @param {string} name - The data source name
 * @param {number} dataSourceType - The data source type (0 = Public, 1 = Personal)
 * @returns {[import('@solana/web3.js').PublicKey, number]} The PDA and bump seed
 */
function getDataSourcePda(programId, authority, name, dataSourceType) {
  const anchor = require('@coral-xyz/anchor');
  
  return anchor.web3.PublicKey.findProgramAddressSync(
    [
      Buffer.from('data_source'),
      authority.toBuffer(),
      toSeed(name),
      Buffer.from([dataSourceType]),
    ],
    programId
  );
}

module.exports = {
  toSeed,
  getFeedPda,
  getDataSourcePda,
};

