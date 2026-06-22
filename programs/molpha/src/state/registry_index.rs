use anchor_lang::prelude::*;

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

    /// Initialize a freshly created registry index slot.
    pub fn init(
        &mut self,
        index: u32,
        node: Pubkey,
        secp256k1_pubkey_x: [u8; 32],
        secp256k1_pubkey_y: [u8; 32],
        bump: u8,
    ) {
        self.index = index;
        self.node = node;
        self.secp256k1_pubkey_x = secp256k1_pubkey_x;
        self.secp256k1_pubkey_y = secp256k1_pubkey_y;
        self.bump = bump;
    }
}
