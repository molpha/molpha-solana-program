use anchor_lang::prelude::*;

use crate::error::MolphaError;
use crate::state::{Plan, PlanType};

#[account]
#[derive(InitSpace)]
pub struct Subscription {
    pub owner: Pubkey,
    pub plan_type: PlanType,
    pub valid_until: i64,
    /// Round quota per plan snapshot. Enforced by the gateway at admission, not on chain.
    pub max_rounds: u64,
    pub delegate_count: u32,
    pub max_delegates: u32,
    pub max_signers: u8,
    pub bump: u8,
}

impl Subscription {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_subscription";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    pub const SECONDS_PER_MONTH: i64 = 30 * 24 * 60 * 60;

    pub fn init(&mut self, owner: Pubkey, plan: &Plan, now: i64, bump: u8) {
        self.owner = owner;
        self.plan_type = plan.plan_type;
        self.valid_until = now
            .checked_add(Self::SECONDS_PER_MONTH)
            .ok_or(MolphaError::InvalidTimestamp)
            .unwrap();
        self.max_rounds = plan.max_rounds;
        self.max_delegates = plan.max_delegates;
        self.max_signers = plan.max_signers;
        self.bump = bump;
    }

    pub fn is_active_at(&self, now: i64) -> bool {
        self.valid_until > now
    }

    pub fn is_active(&self) -> bool {
        self.is_active_at(Clock::get().map(|c| c.unix_timestamp).unwrap_or(0))
    }

    pub fn try_add_delegate(&mut self) -> Result<()> {
        require!(
            self.delegate_count < self.max_delegates,
            crate::error::MolphaError::DelegateCapReached
        );
        self.delegate_count = self
            .delegate_count
            .checked_add(1)
            .ok_or(crate::error::MolphaError::DelegateCapReached)?;
        Ok(())
    }

    pub fn release_delegate(&mut self) {
        self.delegate_count = self.delegate_count.saturating_sub(1);
    }
}
