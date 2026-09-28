//! Permissionless: recognize income that landed in the treasury (x402 payments, subscriptions,
//! externally pulled plans) as pool income of the open epoch.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::{
    error::MolphaError,
    events::RevenueSwept,
    state::{EpochState, ProtocolConfig},
};

/// Unswept income is whatever the treasury holds above `committed()`: claimable rewards, live
/// epoch pools and the protocol reserve. No tokens move; the split is pure accounting.
pub fn sweep_treasury_revenue(ctx: Context<SweepTreasuryRevenue>, open_epoch: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut epoch = ctx.accounts.epoch_state.load_mut()?;
    require!(epoch.is_open(now), MolphaError::EpochNotOpen);

    let config = &mut ctx.accounts.protocol_config;
    let gross = ctx
        .accounts
        .treasury
        .amount
        .checked_sub(config.committed()?)
        .ok_or(MolphaError::TreasuryInsolvent)?;
    require!(gross > 0, MolphaError::NothingToSweep);

    let (protocol_fee, pool_share) = config.record_income(gross)?;
    epoch.pool = epoch
        .pool
        .checked_add(pool_share)
        .ok_or(MolphaError::RewardOverflow)?;

    emit!(RevenueSwept {
        epoch: open_epoch,
        gross,
        protocol_fee,
        pool_share,
        swept_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(open_epoch: u64)]
pub struct SweepTreasuryRevenue<'info> {
    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [EpochState::SEED_PREFIX, open_epoch.to_le_bytes().as_ref()],
        bump = epoch_state.load()?.bump,
    )]
    pub epoch_state: AccountLoader<'info, EpochState>,

    pub token_program: Program<'info, Token>,
}
