use anchor_lang::prelude::*;

use crate::error::MolphaError;

/// Cross-validator clock drift allowance for timestamps slightly ahead of `Clock`.
pub const MAX_FUTURE_DRIFT_SECONDS: u64 = 60;

/// `timestamp` here is unix SECONDS: callers pass `payload.timestamp_secs()`.
///
/// Both bounds inclusive: `now - max_settlement_delay_seconds ..= now + MAX_FUTURE_DRIFT_SECONDS`.
pub fn require_fresh_timestamp(
    timestamp: u64,
    now: i64,
    max_settlement_delay_seconds: u64,
) -> Result<()> {
    // A pre-1970 cluster clock is a broken clock; clamping to 0 fails closed
    // (every real timestamp is then rejected as future).
    let now = u64::try_from(now).unwrap_or(0);
    require!(
        timestamp <= now.saturating_add(MAX_FUTURE_DRIFT_SECONDS),
        MolphaError::AttestationInFuture
    );
    require!(
        timestamp >= now.saturating_sub(max_settlement_delay_seconds),
        MolphaError::AttestationTooStale
    );
    Ok(())
}
