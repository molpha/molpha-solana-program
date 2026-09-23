use anchor_lang::prelude::*;

/// Minimal per-consumer marker that makes settlement idempotency enforceable by PDA creation.
///
/// A marker only has to outlive the freshness window of its round: once
/// `canonical_timestamp + MAX_SETTLEMENT_DELAY_CAP_SECONDS` has passed, no configuration can make
/// the round settleable again, and `close_round_replay` returns the rent to the gateway authority.
#[account]
#[derive(InitSpace)]
pub struct RoundReplayMarker {
    pub recorded_at: i64,
    pub bump: u8,
}

impl RoundReplayMarker {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const X402_SEED_PREFIX: &'static [u8] = b"molpha_x402_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}

/// Which settlement path created a replay marker; selects its seed prefix.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoundReplayKind {
    /// `settle_subscription_round`; `consumer_id` is the subscription owner.
    Subscription,
    /// `settle_x402_round`; `consumer_id` is the rail-scoped `payer_id`.
    X402,
}

impl RoundReplayKind {
    pub const fn seed_prefix(self) -> &'static [u8] {
        match self {
            Self::Subscription => RoundReplayMarker::SEED_PREFIX,
            Self::X402 => RoundReplayMarker::X402_SEED_PREFIX,
        }
    }
}
