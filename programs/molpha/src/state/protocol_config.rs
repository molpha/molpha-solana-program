use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub underlying_token: Pubkey, // SPL token mint for subscription payments

    // EVM-style pricing parameters (from PricingHelper.sol)
    pub base_price_per_second_scaled: u64, // Base price per second * SCALAR
    pub frequency_coefficient: u64,        // Coefficient for frequency scaling (basis points)
    pub signers_coefficient: u64,          // Coefficient for signers scaling (basis points)
    pub reward_percentage: u64,            // Reward percentage in basis points

    // Node security deposit
    pub node_security_deposit: u64,        // Required security deposit amount for registering a node

    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    pub const SCALAR: u64 = 1_000_000; // Same as EVM version
}
