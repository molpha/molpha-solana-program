//! Permissionless: close a fully credited epoch and roll its residual into the open epoch.

use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::EpochClosed, state::EpochState};

/// Requires every weighted slot credited and the evidence window (one epoch length after the
/// ticket deadline) elapsed, so inactivity evidence can still read the account. The residual —
/// forfeited shares, rounding dust, or the whole pool when no node had weight — joins the pool
/// of the epoch open now; `pool_liability` is unchanged. Rent returns to whoever created it.
pub fn close_epoch(ctx: Context<CloseEpoch>, epoch: u64, open_epoch: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let residual = {
        let state = ctx.accounts.epoch_state.load()?;
        require!(state.closable(now), MolphaError::EpochNotClosable);
        state.residual()
    };

    let mut open = ctx.accounts.open_epoch_state.load_mut()?;
    require!(open.is_open(now), MolphaError::EpochNotOpen);
    open.pool = open
        .pool
        .checked_add(residual)
        .ok_or(MolphaError::RewardOverflow)?;

    emit!(EpochClosed {
        epoch,
        residual,
        rolled_into: open_epoch,
        closed_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(epoch: u64, open_epoch: u64)]
pub struct CloseEpoch<'info> {
    #[account(
        mut,
        close = rent_payer,
        seeds = [EpochState::SEED_PREFIX, epoch.to_le_bytes().as_ref()],
        bump = epoch_state.load()?.bump,
    )]
    pub epoch_state: AccountLoader<'info, EpochState>,

    #[account(
        mut,
        seeds = [EpochState::SEED_PREFIX, open_epoch.to_le_bytes().as_ref()],
        bump = open_epoch_state.load()?.bump,
    )]
    pub open_epoch_state: AccountLoader<'info, EpochState>,

    /// CHECK: receives the closed account's rent; must be the account that created it.
    #[account(
        mut,
        address = Pubkey::new_from_array(epoch_state.load()?.rent_payer) @ MolphaError::InvalidRentReceiver
    )]
    pub rent_payer: UncheckedAccount<'info>,
}
