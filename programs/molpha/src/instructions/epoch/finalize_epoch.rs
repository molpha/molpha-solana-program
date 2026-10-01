//! Permissionless: freeze an epoch's payout weights and create the epoch two ahead.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{EpochCreated, EpochFinalized},
    state::{EpochState, ProtocolConfig},
};

/// Callable once the epoch's ticket window (`end + ticket_grace`) has passed. Epochs are created
/// two ahead so a sweep or ticket always finds its account; `e + 2` starts where `e + 1` ends and
/// takes the current config, so parameter changes never reach an existing epoch. Finalization is
/// sequential by construction: `e + 1` exists only once `e - 1` was finalized.
pub fn finalize_epoch(ctx: Context<FinalizeEpoch>, epoch: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.protocol_config;
    let successor_index = epoch
        .checked_add(2)
        .ok_or(MolphaError::InvalidEpochSequence)?;

    let mut target = ctx.accounts.target.load_mut()?;
    require!(target.finalizable(now), MolphaError::EpochNotFinalizable);
    let (weight_total, weighted_count) = target.finalize();

    let next = ctx.accounts.next.load()?;
    // The successor must outlast `next`'s ticket grace, otherwise `next` could only be finalized
    // after the successor ended and the epoch after it would open late.
    let len = config.epoch_len_seconds.max(next.ticket_grace as u64 + 1) as i64;
    let start = next.end;
    let end = start
        .checked_add(len)
        .ok_or(MolphaError::InvalidEpochLength)?;
    let ticket_grace = config.ticket_grace_seconds as i64;
    let tau = target.next_tau(config.target_tickets_per_epoch);

    ctx.accounts.successor.load_init()?.init(
        successor_index,
        start,
        end,
        ticket_grace,
        tau,
        config.min_availability_bps,
        config.min_availability_samples,
        config.min_signers,
        ctx.accounts.payer.key(),
        ctx.bumps.successor,
    );

    emit!(EpochFinalized {
        epoch,
        pool: target.pool,
        tickets: target.tickets,
        roster_len: target.roster_len,
        weight_total,
        weighted_count,
        finalized_at: now,
    });
    emit!(EpochCreated {
        epoch: successor_index,
        start,
        end,
        ticket_grace,
        tau,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct FinalizeEpoch<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(seeds = [ProtocolConfig::SEED_PREFIX], bump = protocol_config.bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [EpochState::SEED_PREFIX, epoch.to_le_bytes().as_ref()],
        bump = target.load()?.bump,
    )]
    pub target: AccountLoader<'info, EpochState>,

    #[account(
        seeds = [EpochState::SEED_PREFIX, epoch.saturating_add(1).to_le_bytes().as_ref()],
        bump = next.load()?.bump,
    )]
    pub next: AccountLoader<'info, EpochState>,

    #[account(
        init,
        payer = payer,
        space = EpochState::SPACE,
        seeds = [EpochState::SEED_PREFIX, epoch.saturating_add(2).to_le_bytes().as_ref()],
        bump,
    )]
    pub successor: AccountLoader<'info, EpochState>,

    pub system_program: Program<'info, System>,
}
