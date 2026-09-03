//! Withdraw a cooled-down node security deposit without closing the permanent Node account.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::NodeDepositWithdrawn,
    state::{Node, NodeStatus, ProtocolConfig, NODE_DEPOSITS_SEED},
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

    // The vault is shared, so the payout is bounded explicitly by this node's own entitlement
    // rather than by the balance of an account only this node can draw on.
    require!(
        amount <= ctx.accounts.node.locked_amount,
        MolphaError::NothingToWithdraw
    );
    require!(
        ctx.accounts.node_deposit_vault.amount >= amount,
        MolphaError::DepositVaultUnderfunded
    );

    ctx.accounts.node.locked_amount = 0;
    let config = &mut ctx.accounts.protocol_config;
    config.total_node_deposits = config
        .total_node_deposits
        .checked_sub(amount)
        .ok_or(MolphaError::DepositVaultUnderfunded)?;

    let vault_bump = [config.node_deposits_bump];
    let signer_seeds: &[&[u8]] = &[NODE_DEPOSITS_SEED, &vault_bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.node_deposit_vault.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.node_deposits.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;

    // The vault still covers every other node's deposit.
    ctx.accounts.node_deposit_vault.reload()?;
    require!(
        ctx.accounts.node_deposit_vault.amount >= ctx.accounts.protocol_config.total_node_deposits,
        MolphaError::DepositVaultUnderfunded
    );

    let node = &ctx.accounts.node;

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

    /// CHECK: seeds-validated deposit-vault authority; holds no data.
    #[account(
        seeds = [NODE_DEPOSITS_SEED],
        bump = protocol_config.node_deposits_bump,
    )]
    pub node_deposits: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = node_deposits,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = owner,
        associated_token::token_program = token_program,
    )]
    pub destination: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
