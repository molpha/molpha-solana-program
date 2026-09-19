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
    pub max_signers: u8,
    /// Max delegates/agents an owner on this plan may provision.
    pub max_delegates: u32,
    pub max_rounds: u64,
    pub private_api_enabled: bool,
    pub is_active: bool,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PlanConfig {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_signers: u8,
    pub max_delegates: u32,
    pub max_rounds: u64,
    pub private_api_enabled: bool,
    pub is_active: bool,
}
impl Plan {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_plan";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, config: &PlanConfig, bump: u8) {
        self.plan_type = config.plan_type;
        self.subscription_price = config.subscription_price;
        self.max_signers = config.max_signers;
        self.max_delegates = config.max_delegates;
        self.private_api_enabled = config.private_api_enabled;
        self.is_active = config.is_active;
        self.max_rounds = config.max_rounds;
        self.bump = bump;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, config: &PlanConfig) {
        self.subscription_price = config.subscription_price;
        self.max_signers = config.max_signers;
        self.max_delegates = config.max_delegates;
        self.max_rounds = config.max_rounds;
        self.private_api_enabled = config.private_api_enabled;
        self.is_active = config.is_active;
        self.max_rounds = config.max_rounds;
    }
}
