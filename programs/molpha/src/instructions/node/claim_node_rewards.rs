//! Withdraw epoch rewards credited to a permanent Node account.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::RewardsClaimed,
    state::{Node, ProtocolConfig},
};

pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
    let amount = ctx.accounts.node.claimable_rewards;
    require!(amount > 0, MolphaError::NothingToClaim);
    require!(
        ctx.accounts.treasury.amount >= amount,
        MolphaError::TreasuryInsolvent
    );

    let config = &mut ctx.accounts.protocol_config;
    config.reward_liability = config
        .reward_liability
        .checked_sub(amount)
        .ok_or(MolphaError::RewardLiabilityUnderflow)?;
    ctx.accounts.node.claimable_rewards = 0;

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

    emit!(RewardsClaimed {
        beneficiary: ctx.accounts.node.authority,
        amount,
        claimed_at: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimNodeRewards<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        has_one = authority,
        seeds = [Node::SEED_PREFIX, authority.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
