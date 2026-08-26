use anchor_lang::prelude::*;

/// Per-round settlement record. Two funding paths write this record:
/// - Subscription draw, via `settle_subscription_round`;
///   seed `["molpha_round", subscription.owner, source_id, signatures_required, timestamp_le]`.
/// - Gateway-funded x402 settlement, via `settle_x402_round`;
///   seed `["molpha_x402_round", payer_id, gateway, source_id, signatures_required, canonical_timestamp_le]`.
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    pub signers_bitmap: [u8; 32],
    pub agg_sig_s: [u8; 32],
    pub commitment: [u8; 20],
    pub attestation_message_hash: [u8; 32],
    pub gateway: Pubkey,
    pub registry_version: u32,
    pub reward_per_signature: u64,
    pub gateway_fee: u64,
    pub recorded_slot: u64,
    pub status: RoundRecordStatus,
    pub kind: RoundRecordKind,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
#[repr(u8)]
pub enum RoundRecordStatus {
    Pending,
    Finalized,
    Voided,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
#[repr(u8)]
pub enum RoundRecordKind {
    Subscription,
    X402,
}

impl RoundReceiptRecord {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const X402_SEED_PREFIX: &'static [u8] = b"molpha_x402_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
