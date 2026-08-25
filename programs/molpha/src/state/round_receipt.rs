use anchor_lang::prelude::*;

/// Per-round settlement record. Two funding paths write this record:
/// - Subscription draw, via `settle_subscription_round`;
///   seed `["molpha_round", subscription.owner, source_id, signatures_required, timestamp_le]`.
/// - Gateway-funded x402 settlement, via `settle_x402_round`;
///   seed `["molpha_x402_round", payer_id, gateway, source_id, signatures_required, canonical_timestamp_le]`.
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    pub source_id: [u8; 32],
    pub timestamp: u64,
    pub signatures_required: u8,
    pub signers_bitmap: [u8; 32],
    pub agg_sig_hash: [u8; 32],
    pub attestation_message_hash: [u8; 32],
    pub gateway: Pubkey,
    pub registry_version: u32,
    pub reward_per_signature: u64,
    pub gateway_fee: u64,
    pub recorded_slot: u64,
    pub status: RoundRecordStatus,
    pub price: u64,
    /// Funding-path discriminator, explicit rather than inferred from `price > 0` — a zero-priced
    /// x402 round (misconfigured pricing) must still dispute on the x402 path.
    pub kind: RoundRecordKind,
    /// Rail-scoped payer identity (`keccak256(rail_id_be || payer_address_bytes)`).
    /// Zeroed on subscription-path receipts.
    pub payer_id: [u8; 32],
    /// Payment rail registry id (0 = subscription/none). Attested only, never interpreted on-chain.
    pub payment_rail: u16,
    pub payment_ref_len: u8,
    /// Opaque payment reference (tx hash, facilitator receipt, ...), zero-padded. Attested only.
    pub payment_ref: [u8; 64],
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

    /// The valid prefix of `payment_ref`, per `payment_ref_len`.
    pub fn payment_ref_bytes(&self) -> &[u8] {
        &self.payment_ref[..self.payment_ref_len as usize]
    }
}
