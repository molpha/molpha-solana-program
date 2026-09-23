//! Permissionless rent reclaim for settlement replay markers.
//!
//! A marker only guards against settling the same round twice. Settlement rejects any
//! `canonical_timestamp` older than `now - max_settlement_delay_seconds`, and
//! `ProtocolConfig::validate` caps that delay at [`MAX_SETTLEMENT_DELAY_CAP_SECONDS`]. Once
//! `timestamp + MAX_SETTLEMENT_DELAY_CAP_SECONDS < now`, no configuration can make the round
//! settleable again, so the marker can be closed and its rent returned to the gateway authority
//! that paid for it.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::RoundReplayClosed,
    state::{Gateway, RoundReplayMarker, MAX_SETTLEMENT_DELAY_CAP_SECONDS},
};

pub fn close_round_replay(ctx: Context<CloseRoundReplay>) -> Result<()> {
    let round_replay = &ctx.accounts.round_replay;
    let clock = Clock::get()?;
    // A pre-1970 cluster clock is broken; clamping to 0 fails closed.
    let now = u64::try_from(clock.unix_timestamp).unwrap_or(0);

    require!(
        round_replay
            .timestamp
            .saturating_add(MAX_SETTLEMENT_DELAY_CAP_SECONDS)
            < now,
        MolphaError::RoundReplayNotExpired
    );

    emit!(RoundReplayClosed {
        round_replay: round_replay.key(),
        round_id: round_replay.round_id,
        gateway: ctx.accounts.gateway.key(),
        rent_receiver: ctx.accounts.rent_receiver.key(),
        canonical_timestamp: round_replay.timestamp,
        closed_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct CloseRoundReplay<'info> {
    /// Gateway the round was settled through; part of the marker seeds.
    pub gateway: Account<'info, Gateway>,

    /// CHECK: rent destination, pinned to the gateway authority that paid for the marker.
    #[account(
        mut,
        address = gateway.authority @ MolphaError::InvalidRentReceiver,
    )]
    pub rent_receiver: UncheckedAccount<'info>,

    /// The seeds bind the marker to `gateway`: without them, a marker settled through one gateway
    /// could be closed against another and its rent paid to the wrong authority.
    #[account(
        mut,
        close = rent_receiver,
        seeds = [
            RoundReplayMarker::SEED_PREFIX,
            gateway.key().as_ref(),
            round_replay.round_id.as_ref(),
        ],
        bump = round_replay.bump,
    )]
    pub round_replay: Account<'info, RoundReplayMarker>,
}
