use anchor_lang::prelude::*;

use libsecp256k1::PublicKey;

#[account]
#[derive(InitSpace)]
pub struct RegistryIndex {
    pub index: u32,
    pub node: Pubkey,
    pub secp256k1_pubkey_x: [u8; 32],
    pub secp256k1_pubkey_y: [u8; 32],
    pub bump: u8,
}

impl RegistryIndex {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry_index";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
