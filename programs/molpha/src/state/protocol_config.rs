use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub enum ConfigChange {
    RewardPerSignature(u64),
    GatewayFee(u64),
    ReceiptSubmitterBonus(u64),
    ProtocolFeeBps(u16),
    MaxReceiptDelaySlots(u64),
    MinimumNodeDeposit(u64),
}

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub pending_authority: Option<Pubkey>,
    pub usdc_mint: Pubkey,
    pub treasury: Pubkey,
    pub reward_per_signature: u64,
    pub gateway_fee: u64,
    pub receipt_submitter_bonus: u64,
    pub protocol_fee_bps: u16,
    pub max_receipt_delay_slots: u64,
    /// Fixed protocol fee per settled round (v0.2 §4.2 / §6).
    pub protocol_fee: u64,
    /// Slots a settled round must wait before `finalize_round` can run (dispute window).
    pub dispute_window_slots: u64,
    /// Minimum USDC bond a gateway must post to settle receipts.
    pub gateway_bond_min: u64,
    /// Challenger reward, in bps of the slashed bond, on a successful dispute.
    pub challenger_bounty_bps: u16,
    /// Monotonic counter assigning `gateway_index` to bonded gateways.
    pub gateway_count: u32,
    pub registry_cooldown_secs: i64,
    /// Grace period after activation during which the previous registry version remains valid.
    pub previous_grace_secs: i64,
    pub max_node_count: u32,
    pub redundancy_buffer: u8,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_secs: i64,
    pub timelock_expires_at: Option<i64>,
    pub pending_change: Option<ConfigChange>,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
