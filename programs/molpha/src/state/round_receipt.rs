use anchor_lang::prelude::*;

/// Per-round settlement record written by `settle_subscription_round` or `settle_x402_round`.
#[account]
#[derive(InitSpace)]
pub struct RoundReceiptRecord {
    // for x402 it's payer_id, for subscription it's subcription owner pubkey
    pub consumer_id: [u8; 32],
    pub signers_bitmap: [u8; 32],
    pub agg_sig_s: [u8; 32],
    pub commitment: [u8; 20],
    pub attestation_message_hash: [u8; 32],
    pub gateway: Pubkey,
    pub registry_version: u32,
    pub reward_per_signature: u64,
    pub recorded_slot: u64,
    pub status: RoundRecordStatus,
    pub kind: RoundRecordKind,
    pub bump: u8,
}

#[derive(InitSpace, AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
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

pub struct RoundReceiptInitArgs {
    pub consumer_id: [u8; 32],
    pub agg_sig: crate::state::AggregatedSig,
    pub attestation_message_hash: [u8; 32],
    pub gateway: Pubkey,
    pub registry_version: u32,
    pub reward_per_signature: u64,
    pub recorded_slot: u64,
    pub kind: RoundRecordKind,
    pub bump: u8,
}

impl RoundReceiptRecord {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const X402_SEED_PREFIX: &'static [u8] = b"molpha_x402_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, args: RoundReceiptInitArgs) {
        self.consumer_id = args.consumer_id;
        self.signers_bitmap = args.agg_sig.signers_bitmap;
        self.agg_sig_s = args.agg_sig.agg_sig_s;
        self.commitment = args.agg_sig.commitment;
        self.attestation_message_hash = args.attestation_message_hash;
        self.gateway = args.gateway;
        self.registry_version = args.registry_version;
        self.reward_per_signature = args.reward_per_signature;
        self.recorded_slot = args.recorded_slot;
        self.status = RoundRecordStatus::Pending;
        self.kind = args.kind;
        self.bump = args.bump;
    }
}
