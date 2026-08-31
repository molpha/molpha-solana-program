//! Withdraw a cooled-down node security deposit without closing the permanent Node account.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::NodeDepositWithdrawn,
    state::{Node, NodeStatus, ProtocolConfig},
};

pub fn withdraw_node_deposit(ctx: Context<WithdrawNodeDeposit>) -> Result<()> {
    let node = &ctx.accounts.node;
    let amount = node.locked_amount;
    require!(amount > 0, MolphaError::NothingToWithdraw);
    require!(
        node.status == NodeStatus::Deactivated || node.status == NodeStatus::Tombstoned,
        MolphaError::InvalidWithdrawalState
    );
    require!(
        node.withdrawable_slot > 0,
        MolphaError::InvalidWithdrawalState
    );

    let clock = Clock::get()?;
    require!(
        clock.slot >= node.withdrawable_slot,
        MolphaError::WithdrawalCooldownActive
    );

    let config = &ctx.accounts.protocol_config;
    let bump = [config.bump];
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

    let node = &mut ctx.accounts.node;
    node.locked_amount = 0;

    emit!(NodeDepositWithdrawn {
        node: node.key(),
        owner: node.owner,
        destination: ctx.accounts.destination.key(),
        amount,
        withdrawn_at: clock.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawNodeDeposit<'info> {
    #[account(mut)]
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
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidMint
    )]
    pub usdc_mint: Account<'info, Mint>,

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
