use anchor_lang::prelude::*;

use crate::error::MolphaError;
use crate::state::PlanType;

#[account]
#[derive(InitSpace)]
pub struct Subscription {
    pub owner: Pubkey,
    pub plan_type: PlanType,
    pub prepaid_usdc: u64,
    pub price: u64,
    pub valid_until: i64,
    pub used_rounds: u64,
    pub max_rounds: u64,
    pub delegate_count: u32,
    pub max_delegates: u32,
    pub max_signers: u8,
    pub bump: u8,
}

pub struct SubscriptionInitArgs {
    pub owner: Pubkey,
    pub plan_type: PlanType,
    pub monthly_price: u64,
    pub max_delegates: u32,
    pub max_signers: u8,
    pub max_rounds: u64,
    pub now: i64,
    pub bump: u8,
}

impl Subscription {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_subscription";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    pub const SECONDS_PER_MONTH: i64 = 30 * 24 * 60 * 60;

    pub fn init(&mut self, args: SubscriptionInitArgs) {
        self.owner = args.owner;
        self.plan_type = args.plan_type;
        self.prepaid_usdc = args.monthly_price;
        self.price = args.monthly_price;
        self.valid_until = args
            .now
            .checked_add(Self::SECONDS_PER_MONTH)
            .ok_or(MolphaError::InvalidTimestamp)
            .unwrap();
        self.used_rounds = 0;
        self.max_rounds = args.max_rounds;
        self.delegate_count = 0;
        self.max_delegates = args.max_delegates;
        self.max_signers = args.max_signers;
        self.bump = args.bump;
    }

    pub fn is_active_at(&self, now: i64) -> bool {
        self.valid_until > now
    }

    pub fn is_active(&self) -> bool {
        self.is_active_at(Clock::get().map(|c| c.unix_timestamp).unwrap_or(0))
    }

    pub fn has_round_quota(&self) -> bool {
        self.used_rounds < self.max_rounds
    }

    pub fn consume_round(&mut self) -> Result<()> {
        self.used_rounds = self
            .used_rounds
            .checked_add(1)
            .ok_or(crate::error::MolphaError::SubscriptionInactiveOrExhausted)?;
        Ok(())
    }

    /// Undo `consume_round` when a pending round is voided by `validate_round`.
    pub fn restore_round(&mut self) -> Result<()> {
        require!(
            self.used_rounds > 0,
            crate::error::MolphaError::SubscriptionInactiveOrExhausted
        );
        self.used_rounds -= 1;
        Ok(())
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
