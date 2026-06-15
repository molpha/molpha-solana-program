use anchor_lang::prelude::*;

use crate::state::PlanType;

#[account]
#[derive(InitSpace)]
pub struct Subscription {
    pub owner: Pubkey,
    pub plan_type: PlanType,
    pub prepaid_usdc: u64,
    pub price: u64,
    pub valid_until: i64,
    pub job_count: u32,
    pub bump: u8,
}

impl Subscription {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_subscription";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    pub const SECONDS_PER_MONTH: i64 = 30 * 24 * 60 * 60;

    pub fn init(
        &mut self,
        owner: Pubkey,
        plan_type: PlanType,
        monthly_price: u64,
        now: i64,
        bump: u8,
    ) {
        self.owner = owner;
        self.plan_type = plan_type;
        self.prepaid_usdc = monthly_price;
        self.price = monthly_price;
        self.valid_until = now.checked_add(Self::SECONDS_PER_MONTH).unwrap_or(i64::MAX);
        self.job_count = 0;
        self.bump = bump;
    }

    pub fn is_active(&self) -> bool {
        self.valid_until > Clock::get().map(|c| c.unix_timestamp).unwrap_or(0)
    }
}
