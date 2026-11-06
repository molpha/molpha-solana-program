use anchor_lang::prelude::*;

/// Permanent reservation for the x coordinate of a secp256k1 signing key, held by the node or
/// provider the key belongs to.
///
/// PDA `["molpha_key_x", x]`. The claim is on `x` alone, so a key and its negation (same `x`,
/// other parity) are one key: a duplicate or negated key is refused at registration, in O(1). A
/// rotated-away key keeps its claim, so a retired key can never be registered again.
#[account]
#[derive(InitSpace)]
pub struct KeyXClaim {
    /// The `Node` or `Provider` PDA that holds the key.
    pub owner: Pubkey,
    pub bump: u8,
}

impl KeyXClaim {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_key_x";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, owner: Pubkey, bump: u8) {
        self.owner = owner;
        self.bump = bump;
    }
}

/// The 32-byte big-endian x coordinate of a compressed secp256k1 key.
pub fn compressed_key_x(compressed: &[u8; 33]) -> &[u8] {
    &compressed[1..33]
}
