use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::RoundReplayClosed,
    state::{Gateway, RoundReplayMarker, MAX_SETTLEMENT_DELAY_CAP_SECONDS},
};

pub fn close_round_replay(ctx: Context<CloseRoundReplay>) -> Result<()> {
    let round_replay = &ctx.accounts.round_replay;

    let clock = Clock::get()?;
    require!(
        round_replay
            .recorded_at
            .saturating_add(MAX_SETTLEMENT_DELAY_CAP_SECONDS as i64)
            < clock.unix_timestamp,
        MolphaError::RoundReplayNotExpired
    );

    emit!(RoundReplayClosed {
        round_replay: round_replay.key(),
        gateway: ctx.accounts.gateway.key(),
        rent_receiver: ctx.accounts.rent_receiver.key(),
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

    /// Seeds are verified in the handler (the prefix depends on `args.kind`).
    #[account(mut, close = rent_receiver)]
    pub round_replay: Account<'info, RoundReplayMarker>,
}
