use anchor_lang::prelude::*;

use crate::state::AgentStatus;

/// Owner-scoped delegate authorization with persistent request quotas.
#[account]
#[derive(InitSpace)]
pub struct Delegate {
    pub owner: Pubkey,
    pub delegate: Pubkey,
    /// Owner pause / protocol freeze backstop.
    pub status: AgentStatus,
    pub max_data_requests: u64,
    pub data_requests_used: u64,
    /// Per-delegate quorum ceiling; clamped to `plan.max_signers` at creation.
    pub max_signers: u8,
    /// Dormant: per-epoch credit ceiling for the future credit economy. Not enforced.
    pub epoch_cap: u64,
    /// Dormant: credits spent in the current epoch. Not enforced.
    pub epoch_spent: u64,
    /// Dormant: epoch discriminator for `epoch_spent`. Not enforced.
    pub epoch_id: i64,
    pub bump: u8,
}

impl Delegate {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_delegate";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        owner: Pubkey,
        delegate: Pubkey,
        max_data_requests: u64,
        max_signers: u8,
        bump: u8,
    ) {
        self.owner = owner;
        self.delegate = delegate;
        self.status = AgentStatus::Active;
        self.max_data_requests = max_data_requests;
        self.data_requests_used = 0;
        self.max_signers = max_signers;
        self.epoch_cap = 0;
        self.epoch_spent = 0;
        self.epoch_id = 0;
        self.bump = bump;
    }

    pub fn is_active(&self) -> bool {
        self.status == AgentStatus::Active
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
