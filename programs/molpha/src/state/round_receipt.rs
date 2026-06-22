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
    pub selection_bitmap: [u8; 32],
    /// Snapshot of `job.signatures_required` at settlement (dispute message hash).
    pub signatures_required: u8,
    /// `= agg_sig.signers_bitmap` (the credited set).
    pub credited_bitmap: [u8; 32],
    /// `sha256(node_0 || node_1 || ...)` in ascending credited-bit order at settlement.
    pub credited_nodes_hash: [u8; 32],
    /// `= sha256(AggregatedSig)`; the commitment used by the dispute path.
    pub agg_sig_hash: [u8; 32],
    /// Committed `compute_message_hash(..)` used for dispute-time aggregate verification.
    pub data_update_message_hash: [u8; 32],
    pub gateway_index: u32,
    /// The bonded gateway account credited / slashable for this round.
    pub gateway: Pubkey,
    pub registry_version: u32,
    /// Snapshot of `reward_per_signature` at settlement time.
    pub reward_per_signature: u64,
    /// Snapshot of `gateway_fee` at settlement time.
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
