use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::AgentCreditsDeposited,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

/// Permissionless funding: anyone may top up an owner-provisioned agent escrow (owner,
/// sponsor, the agent's own runtime wallet). USDC lands as real funds in the escrow's own
/// ATA — the escrow PDA signs the transfer out to treasury at settlement. (self-funded escrows have no data account and are funded by paying directly to their ATA.)
/// escrows have no data account and are funded by paying directly to their ATA.)
pub fn deposit_agent_credits(ctx: Context<DepositAgentCredits>, amount: u64) -> Result<()> {
    require!(amount > 0, MolphaError::InvalidAmount);
    require!(
        ctx.accounts.agent_escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.depositor_usdc.to_account_info(),
                to: ctx.accounts.escrow_ata.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        amount,
    )?;

    ctx.accounts.escrow_ata.reload()?;

    emit!(AgentCreditsDeposited {
        agent: ctx.accounts.agent_escrow.key(),
        depositor: ctx.accounts.depositor.key(),
        amount,
        new_balance: ctx.accounts.escrow_ata.amount,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct DepositAgentCredits<'info> {
    pub depositor: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub agent_escrow: Account<'info, AgentEscrow>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(
        mut,
        constraint = depositor_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = depositor_usdc.owner == depositor.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub depositor_usdc: Account<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = agent_escrow,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
