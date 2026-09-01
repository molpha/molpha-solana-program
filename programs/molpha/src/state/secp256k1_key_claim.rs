use anchor_lang::prelude::*;

/// Permanent reservation for a secp256k1 node key.
#[account]
#[derive(InitSpace)]
pub struct Secp256k1KeyClaim {
    pub node_pubkey: Pubkey,
    pub bump: u8,
}

impl Secp256k1KeyClaim {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_secp_key";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, node_pubkey: Pubkey, bump: u8) {
        self.node_pubkey = node_pubkey;
        self.bump = bump;
    }
}
