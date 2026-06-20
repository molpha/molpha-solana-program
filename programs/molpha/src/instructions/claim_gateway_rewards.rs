//! `claim_gateway_rewards` — withdraw a gateway's finalized fees.
//!
//! Mirrors `claim_node_rewards`: only `claimable_rewards` (promoted by `finalize_round`) are
//! withdrawable. The gateway's bond stays escrowed in the treasury.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::RewardsClaimed,
    state::{Gateway, ProtocolConfig},
};

pub fn claim_gateway_rewards(ctx: Context<ClaimGatewayRewards>) -> Result<()> {
    let amount = ctx.accounts.gateway.claimable_rewards;
    require!(amount > 0, MolphaError::NothingToClaim);

    let config = &ctx.accounts.protocol_config;
    let bump = [config.bump];
    let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.treasury.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.protocol_config.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;

    ctx.accounts.gateway.claimable_rewards = 0;

    emit!(RewardsClaimed {
        beneficiary: ctx.accounts.gateway.authority,
        destination: ctx.accounts.destination.key(),
        amount,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimGatewayRewards<'info> {
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
        constraint = gateway.authority == authority.key() @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = destination.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = destination.owner == authority.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
