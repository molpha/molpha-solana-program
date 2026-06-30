use anchor_lang::prelude::*;

/// Per-round settlement record (one per `(job_id, timestamp)`).
///
/// Created optimistically by `settle_round_receipt` without verifying the committed
/// `AggregatedSig`. Settlement also commits the exact data-update message hash used for any
/// future dispute verification. Credits become withdrawable once `finalized == true`
/// (after the dispute window) via `finalize_round`.
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    pub job_id: [u8; 32],
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
    /// Credits moved from `pending_rewards` to `claimable_rewards`.
    pub finalized: bool,
    /// Successfully disputed — credits are voided and never finalize.
    pub voided: bool,
    pub bump: u8,
}

impl RoundReceiptRecord {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
