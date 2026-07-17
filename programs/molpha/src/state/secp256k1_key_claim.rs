use anchor_lang::prelude::*;

/// Permanent reservation for a secp256k1 node key.
///
/// The PDA is keyed by `keccak256(compressed_pubkey)`. Claims are intentionally never closed:
/// once a key has represented a Molpha node it cannot later be replayed for another identity.
#[account]
#[derive(InitSpace)]
pub struct Secp256k1KeyClaim {
    pub claimed: bool,
    pub key_fingerprint: [u8; 32],
    pub secp256k1_pubkey_compressed: [u8; 33],
    pub owner: Pubkey,
    pub node_id: [u8; 32],
    pub bump: u8,
}

impl Secp256k1KeyClaim {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_secp_key";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        key_fingerprint: [u8; 32],
        secp256k1_pubkey_compressed: [u8; 33],
        owner: Pubkey,
        node_id: [u8; 32],
        bump: u8,
    ) {
        self.claimed = true;
        self.key_fingerprint = key_fingerprint;
        self.secp256k1_pubkey_compressed = secp256k1_pubkey_compressed;
        self.owner = owner;
        self.node_id = node_id;
        self.bump = bump;
    }
}
