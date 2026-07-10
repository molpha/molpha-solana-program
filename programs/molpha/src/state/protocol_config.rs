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
    pub registry_cooldown_secs: i64,
    /// Grace period after activation during which the previous registry version remains valid.
    pub previous_grace_secs: i64,
    pub max_node_count: u32,
    pub redundancy_buffer: u8,
    pub minimum_node_deposit: u64,
    pub withdrawal_cooldown_secs: i64,
    // DEAD STATE: `timelock_expires_at` / `pending_change` (and the `ConfigChange` enum)
    // have no propose/apply instruction — they are initialized to `None` and never read.
    // Kept for layout stability until a config-update path is designed.
    pub timelock_expires_at: Option<i64>,
    pub pending_change: Option<ConfigChange>,
    // ---- Agent pay-per-request pricing and policy (agent-access spec v1). ----
    /// Base price of an agent `FreshRound` request (USDC atomic units).
    pub agent_round_base: u64,
    /// Per-required-signature price component of a `FreshRound` request.
    /// Solvency invariant (ops-enforced): `agent_round_per_sig >= reward_per_signature`
    /// and `agent_round_base >= redundancy_buffer * reward_per_signature + gateway_fee`.
    pub agent_round_per_sig: u64,
    /// Flat price of an agent `CachedRead` request (gateway fee + protocol margin).
    pub agent_cached_read_price: u64,
    /// Length of the agent spend epoch (e.g. 86_400 for daily); must be > 0.
    pub agent_epoch_len_secs: i64,
    pub bump: u8,
}

impl ProtocolConfig {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_config";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
