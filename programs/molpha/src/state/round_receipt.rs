use anchor_lang::prelude::*;

/// Subscription round settled by `settle_round_receipt`; seed `["molpha_round", feed_id, ts_le]`.
pub const RECORD_KIND_SUBSCRIPTION: u8 = 0;
/// Agent `FreshRound` settled by `settle_agent_round`; dispute-exposed like a subscription round.
pub const RECORD_KIND_AGENT_FRESH: u8 = 1;

/// Per-round settlement record.
///
/// One per `(feed_id, timestamp)` for subscription rounds, one per `(agent, request_nonce)`
/// for agent rounds (seed `AGENT_SEED_PREFIX`). Created optimistically without verifying the
/// committed `AggregatedSig`. Settlement also commits the exact data-update message hash used
/// for any future dispute verification. Credits become withdrawable once `finalized == true`
/// (after the dispute window) via `finalize_round`.
///
/// `finalize_round` / `dispute_round` accept both variants: they validate the account by
/// owner + discriminator and trust stored contents (only the two settle instructions can
/// create records, and both write complete state).
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    pub feed_id: [u8; 32],
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
    pub finalized: bool,
    pub voided: bool,
    pub price: u64,
    pub bump: u8,
}

impl RoundReceiptRecord {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    /// Agent-round records: `[AGENT_SEED_PREFIX, agent, request_nonce_le]`.
    pub const AGENT_SEED_PREFIX: &'static [u8] = b"molpha_agent_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
