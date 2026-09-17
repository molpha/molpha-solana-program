//! Withdraw a cooled-down node security deposit without closing the permanent Node account.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Token, TokenAccount, Transfer},
};

use crate::{
    error::MolphaError,
    events::NodeDepositWithdrawn,
    state::{Node, NodeStatus, ProtocolConfig, RegistryState},
};

pub fn withdraw_node_deposit(ctx: Context<WithdrawNodeDeposit>) -> Result<()> {
    let node = &mut ctx.accounts.node;
    let amount = node.locked_amount;
    require!(
        amount > 0 && amount <= node.locked_amount,
        MolphaError::NothingToWithdraw
    );
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
    require!(
        ctx.accounts.node_deposit_vault.amount >= amount,
        MolphaError::DepositVaultUnderfunded
    );

    node.locked_amount = 0;
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.node_deposit_vault.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.registry_state.to_account_info(),
            },
            &[&ctx.accounts.registry_state.seeds()],
        ),
        amount,
    )?;

    emit!(NodeDepositWithdrawn {
        node: node.key(),
        node_authority: node.authority,
        destination: ctx.accounts.destination.key(),
        amount,
        withdrawn_at: clock.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawNodeDeposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Node::SEED_PREFIX, authority.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Account<'info, TokenAccount>,

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
