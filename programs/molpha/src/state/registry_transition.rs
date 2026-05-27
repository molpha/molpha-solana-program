use anchor_lang::prelude::*;

use crate::state::RegistryTransitionType;

#[account]
#[derive(InitSpace)]
pub struct RegistryTransitionAccount {
    pub previous_version: u32,
    pub current_version: u32,
    pub expires_at: i64,
    pub transition_type: RegistryTransitionType,
    pub removed_old_index: u32,
    pub removed_node: Pubkey,
    pub moved_old_index: u32,
    pub moved_node: Pubkey,
    pub bump: u8,
}

impl RegistryTransitionAccount {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry_transition";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
