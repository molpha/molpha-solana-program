use anchor_lang::prelude::*;

/// Minimal per-consumer marker that makes settlement idempotency enforceable by PDA creation.
#[account]
#[derive(InitSpace)]
pub struct RoundReplayMarker {
    pub bump: u8,
}

impl RoundReplayMarker {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_round";
    pub const X402_SEED_PREFIX: &'static [u8] = b"molpha_x402_round";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}
