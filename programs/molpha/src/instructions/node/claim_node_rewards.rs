//! Withdraw finalized rewards from a permanent Node account.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::RewardsClaimed,
    state::{Node, ProtocolConfig},
};

pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
    let amount = ctx.accounts.node.claimable_rewards;
    require!(amount > 0, MolphaError::NothingToClaim);

    // Rewards are backed by treasury revenue only; the invariant is maintained at credit time,
    // so a failure here means the treasury was drained outside the protocol's own payout paths.
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

    let bump = [ctx.accounts.protocol_config.bump];
    let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.treasury.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.protocol_config.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;

    emit!(RewardsClaimed {
        beneficiary: ctx.accounts.node.owner,
        amount,
        claimed_at: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimNodeRewards<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Node::SEED_PREFIX, owner.key().as_ref()],
        bump = node.bump,
        constraint = node.owner == owner.key() @ MolphaError::UnauthorizedNodeAuthority,
    )]
    pub node: Account<'info, Node>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidMint
    )]
    pub usdc_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = usdc_mint,
        associated_token::authority = owner,
        associated_token::token_program = token_program,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
