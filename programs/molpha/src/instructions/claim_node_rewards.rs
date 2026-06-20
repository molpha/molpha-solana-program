//! `claim_node_rewards` — withdraw a node's finalized rewards.
//!
//! Only `claimable_rewards` (credits promoted by `finalize_round` after the dispute window) are
//! withdrawable; `pending_rewards` remain locked until finalization. This is the dispute-window
//! finality gate of the spec's `claim()`.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::RewardsClaimed,
    state::{Node, ProtocolConfig},
};

pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
    let amount = ctx.accounts.node.claimable_rewards;
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

    ctx.accounts.node.claimable_rewards = 0;

    emit!(RewardsClaimed {
        beneficiary: ctx.accounts.node.owner,
        destination: ctx.accounts.destination.key(),
        amount,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimNodeRewards<'info> {
    pub owner: Signer<'info>,

    #[account(
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
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = destination.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = destination.owner == owner.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
