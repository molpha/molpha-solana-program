use anchor_lang::prelude::*;

#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
#[repr(u8)]
pub enum PlanType {
    #[default]
    Basic,
    Standard,
    Professional,
    Enterprise,
}

#[account]
#[derive(InitSpace)]
pub struct Plan {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_jobs: u32,
    pub max_signers: u8,
    pub private_api_enabled: bool,
    pub is_active: bool,
    pub bump: u8,
}

pub struct PlanInitParams {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_jobs: u32,
    pub max_signers: u8,
    pub private_api_enabled: bool,
    pub is_active: bool,
    pub bump: u8,
}

impl Plan {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_plan";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, params: &PlanInitParams) {
        self.plan_type = params.plan_type;
        self.subscription_price = params.subscription_price;
        self.max_jobs = params.max_jobs;
        self.max_signers = params.max_signers;
        self.private_api_enabled = params.private_api_enabled;
        self.is_active = params.is_active;
        self.bump = params.bump;
    }

    pub fn update(
        &mut self,
        plan_type: PlanType,
        subscription_price: u64,
        max_jobs: u32,
        max_signers: u8,
        private_api_enabled: bool,
        is_active: bool,
    ) {
        self.plan_type = plan_type;
        self.subscription_price = subscription_price;
        self.max_jobs = max_jobs;
        self.max_signers = max_signers;
        self.private_api_enabled = private_api_enabled;
        self.is_active = is_active;
    }
}
