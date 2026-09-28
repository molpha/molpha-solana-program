//! Protocol authority withdraws protocol fees and slashing proceeds from the treasury.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{error::MolphaError, events::ProtocolReserveWithdrawn, state::ProtocolConfig};

/// Capped at `protocol_reserved`, so node rewards and live epoch pools are never reachable.
pub fn withdraw_protocol_reserve(ctx: Context<WithdrawProtocolReserve>, amount: u64) -> Result<()> {
    let config = &mut ctx.accounts.protocol_config;
    require!(
        amount > 0 && amount <= config.protocol_reserved,
        MolphaError::InsufficientProtocolReserve
    );
    config.protocol_reserved -= amount;

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.treasury.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.protocol_config.to_account_info(),
            },
            &[&ctx.accounts.protocol_config.seeds()],
        ),
        amount,
    )?;

    emit!(ProtocolReserveWithdrawn {
        destination: ctx.accounts.destination.key(),
        amount,
        withdrawn_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawProtocolReserve<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(mut, token::mint = protocol_config.usdc_mint)]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
