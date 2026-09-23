use anchor_lang::prelude::*;

use molpha_verifier::Attestation;
use solana_keccak_hasher::hashv;

pub const DOMAIN_ROUND_ID: &[u8] = b"MOLPHA_ROUND_ID_V1";

/// Per-settlement marker that makes settlement idempotent by PDA creation.
///
/// PDA: `[SEED_PREFIX, gateway, round_id]`. It only has to outlive its round's freshness window:
/// once `timestamp + MAX_SETTLEMENT_DELAY_CAP_SECONDS` has passed, no configuration can make the
/// round settleable again, and `close_round_replay` returns the rent to the gateway authority.
#[account]
#[derive(InitSpace)]
pub struct RoundReplayMarker {
    /// Seed component; stored so `close_round_replay` can re-derive the PDA and bind the marker
    /// to the gateway whose authority paid for it.
    pub round_id: [u8; 32],
    /// The settled round's `canonical_timestamp` (not the settlement time): freshness is judged
    /// against it, so it is what bounds when the round can no longer be re-admitted.
    pub timestamp: u64,
    pub bump: u8,
}

/// Settlement path that created a marker. Part of the round id, so the two paths never share a
/// marker even when an x402 `payer_id` happens to equal a subscription consumer's key bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum RoundReplayKind {
    /// `settle_subscription_round`; the initiator is the signing consumer (owner or delegate).
    Subscription = 0,
    /// `settle_x402_round`; the initiator is the rail-scoped `payer_id`.
    X402 = 1,
}

impl RoundReplayMarker {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(&mut self, round_id: [u8; 32], timestamp: u64, bump: u8) {
        self.round_id = round_id;
        self.timestamp = timestamp;
        self.bump = bump;
    }

    #[inline]
    pub fn round_id(
        kind: RoundReplayKind,
        attestation: &Attestation,
        initiator: &[u8; 32],
    ) -> [u8; 32] {
        hashv(&[
            DOMAIN_ROUND_ID,
            &[kind as u8],
            attestation.payload.source_id.as_ref(),
            &[attestation.payload.signatures_required],
            &attestation.payload.canonical_timestamp.to_le_bytes(),
            initiator.as_ref(),
        ])
        .to_bytes()
    }
}
