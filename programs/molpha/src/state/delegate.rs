use anchor_lang::prelude::*;

/// Lifecycle status shared by `Delegate`.
#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
pub enum AgentStatus {
    #[default]
    Active,
    Paused,
}

/// Owner-scoped delegate authorization with persistent request quotas.
#[account]
#[derive(InitSpace)]
pub struct Delegate {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    pub max_data_requests: u64,
    pub data_requests_used: u64,
    pub bump: u8,
}

impl Delegate {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_delegate";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, owner: Pubkey, delegate: Pubkey, max_data_requests: u64, bump: u8) {
        self.owner = owner;
        self.delegate = delegate;
        self.max_data_requests = max_data_requests;
        self.data_requests_used = 0;
        self.bump = bump;
    }

    pub fn try_consume_data_request(&mut self) -> Result<()> {
        require!(
            self.data_requests_used < self.max_data_requests,
            crate::error::MolphaError::DelegateDataRequestLimitReached
        );
        self.data_requests_used = self
            .data_requests_used
            .checked_add(1)
            .ok_or(crate::error::MolphaError::DelegateDataRequestLimitReached)?;
        Ok(())
    }
}
