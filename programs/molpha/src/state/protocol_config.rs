use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub underlying_token: Pubkey, // SPL token mint for subscription payments

    // EVM-style pricing parameters (from PricingHelper.sol)
    pub base_price_per_second_scaled: u64, // Base price per second * SCALAR
    pub frequency_coefficient: u64,        // Coefficient for frequency scaling (basis points)
    pub signers_coefficient: u64,          // Coefficient for signers scaling (basis points)
    pub reward_percentage: u64,            // Reward percentage in basis points

    // Node security deposit
    pub node_security_deposit: u64, // Required security deposit amount for registering a node

    // Wormhole cross-chain messaging configuration
    pub wormhole_bridge_program: Pubkey, // Wormhole Core Bridge program ID
    #[max_len(10)]
    pub target_chain_ids: Vec<u16>, // Wormhole chain IDs to notify on node registration
    pub registry_x_agg: [u8; 33],   // Compressed secp256k1 aggregate key for pull verification
    pub signatures_required: u8,     // Minimum signers required for verify_answer
    pub node_count: u32,             // Total active nodes, used for bitmap bounds checks
    /// Redundant slots beyond threshold in selected group (`group_size = signatures_required + redundancy_buffer`).
    pub redundancy_buffer: u32,

    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
    pub const SCALAR: u64 = 1_000_000; // Same as EVM version
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            authority: Pubkey::default(),
            underlying_token: Pubkey::default(),
            base_price_per_second_scaled: 0,
            frequency_coefficient: 0,
            signers_coefficient: 0,
            reward_percentage: 0,
            node_security_deposit: 0,
            wormhole_bridge_program: Pubkey::default(),
            target_chain_ids: Vec::new(),
            registry_x_agg: [0u8; 33],
            signatures_required: 0,
            node_count: 0,
            redundancy_buffer: 2,
            bump: 0,
        }
    }
}
