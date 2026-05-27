use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct RegistryState {
    pub authority: Pubkey,
    pub current_version: u32,
    pub previous_version: u32,
    pub previous_expires_at: i64,
    pub current_node_count: u32,
    pub previous_node_count: u32,
    pub last_transition_type: RegistryTransitionType,
    pub transition_account: Pubkey,
    pub grace_secs: i64,
    pub withdrawal_cooldown_secs: i64,
    pub max_node_count: u32,
    pub bump: u8,
}

impl RegistryState {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_registry";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init_genesis(
        &mut self,
        authority: Pubkey,
        grace_secs: i64,
        withdrawal_cooldown_secs: i64,
        max_node_count: u32,
        bump: u8,
    ) {
        self.authority = authority;
        self.current_version = 0;
        self.previous_version = 0;
        self.previous_expires_at = 0;
        self.current_node_count = 0;
        self.previous_node_count = 0;
        self.last_transition_type = RegistryTransitionType::None;
        self.transition_account = Pubkey::default();
        self.grace_secs = grace_secs;
        self.withdrawal_cooldown_secs = withdrawal_cooldown_secs;
        self.max_node_count = max_node_count;
        self.bump = bump;
    }

    pub fn is_accepted_registry_version(&self, version: u32, now: i64) -> bool {
        version == self.current_version
            || (version == self.previous_version && now <= self.previous_expires_at)
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum RegistryTransitionType {
    None,
    Add,
    RemoveTail,
    RemoveSwap,
}
