use anchor_lang::prelude::*;

/// Subscription round settled by `settle_round_receipt`; seed `["molpha_round", source_id, ts_le]`.
pub const RECORD_KIND_SUBSCRIPTION: u8 = 0;
/// Agent `FreshRound` settled by `settle_agent_round`; dispute-exposed like a subscription round.
pub const RECORD_KIND_AGENT_FRESH: u8 = 1;

/// Per-round settlement record.
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    pub source_id: [u8; 32],
    pub timestamp: u64,
    pub signatures_required: u8,
    pub signers_bitmap: [u8; 32],
    pub agg_sig_hash: [u8; 32],
    pub data_update_message_hash: [u8; 32],
    pub gateway: Pubkey,
    pub registry_version: u32,
    pub reward_per_signature: u64,
    pub gateway_fee: u64,
    pub recorded_slot: u64,
    pub status: RoundRecordStatus,
    pub price: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
#[repr(u8)]
pub enum RoundRecordStatus {
    Pending,
    Finalized,
    Voided,
}

impl RoundReceiptRecord {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const AGENT_SEED_PREFIX: &'static [u8] = b"molpha_agent_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
