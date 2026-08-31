//! Complete a clean gateway exit after both withdrawal clocks have elapsed.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, CloseAccount, Mint, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::GatewayBondWithdrawn,
    state::{Gateway, GatewayStatus, ProtocolConfig},
};

pub fn withdraw_gateway_bond(ctx: Context<WithdrawGatewayBond>) -> Result<()> {
    let gateway = &ctx.accounts.gateway;
    require!(
        gateway.status == GatewayStatus::Deactivated && gateway.withdrawable_slot > 0,
        MolphaError::GatewayNotDeactivated
    );

    let clock = Clock::get()?;
    require!(
        clock.slot >= gateway.withdrawable_slot,
        MolphaError::GatewayDisputeWindowActive
    );

    let amount = ctx.accounts.gateway_bond.amount;
    require!(amount > 0, MolphaError::NothingToWithdraw);

    let authority = gateway.authority;
    let bump = [gateway.bump];
    let signer_seeds: &[&[u8]] = &[Gateway::SEED_PREFIX, authority.as_ref(), &bump];
    let signer = &[signer_seeds];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.gateway_bond.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.gateway.to_account_info(),
            },
            signer,
        ),
        amount,
    )?;
    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.gateway_bond.to_account_info(),
            destination: ctx.accounts.authority.to_account_info(),
            authority: ctx.accounts.gateway.to_account_info(),
        },
        signer,
    ))?;

    emit!(GatewayBondWithdrawn {
        gateway: gateway.key(),
        amount,
        withdrawn_slot: clock.slot,
        withdrawn_at: clock.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawGatewayBond<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Gateway::SEED_PREFIX, authority.key().as_ref()],
        bump = gateway.bump,
        has_one = authority,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidMint
    )]
    pub usdc_mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = gateway,
        associated_token::token_program = token_program,
    )]
    pub gateway_bond: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
