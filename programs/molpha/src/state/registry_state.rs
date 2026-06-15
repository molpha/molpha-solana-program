use anchor_lang::prelude::*;

use crate::state::VIRTUAL_INDEX;

#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub authority: Pubkey,
    pub current_node_count: u32,
    pub grace_secs: i64,
    pub redundancy_buffer: u8,
    // version metadata
    pub current_version: u32,
    pub previous_version: u32,
    pub previous_expires_at: i64,
    // transition metadata
    pub previous_node_count: u32,
    pub last_transition_type: RegistryTransitionType,
    pub removed_old_index: u32,
    pub removed_node: Pubkey,
    pub moved_old_index: u32,
    pub moved_node: Pubkey,
    // bump
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init_genesis(
        &mut self,
        authority: Pubkey,
        redundancy_buffer: u8,
        grace_secs: i64,
        bump: u8,
    ) {
        self.authority = authority;
        self.redundancy_buffer = redundancy_buffer;
        self.current_version = 0;
        self.previous_version = 0;
        self.previous_expires_at = 0;
        self.current_node_count = 0;
        self.previous_node_count = 0;
        self.last_transition_type = RegistryTransitionType::None;
        self.clear_remove_transition_metadata();
        self.grace_secs = grace_secs;
        self.bump = bump;
    }

    pub fn is_accepted_registry_version(&self, version: u32, now: i64) -> bool {
        version == self.current_version
            || (version == self.previous_version && now <= self.previous_expires_at)
    }

    pub fn is_remove_transition(&self) -> bool {
        matches!(
            self.last_transition_type,
            RegistryTransitionType::RemoveTail | RegistryTransitionType::RemoveSwap
        )
    }

    pub fn clear_remove_transition_metadata(&mut self) {
        self.removed_old_index = 0;
        self.removed_node = Pubkey::default();
        self.moved_old_index = VIRTUAL_INDEX;
        self.moved_node = Pubkey::default();
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum RegistryTransitionType {
    None,
    Add,
    RemoveTail,
    RemoveSwap,
}
