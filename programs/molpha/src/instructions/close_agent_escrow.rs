use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::{
    error::MolphaError,
    events::AgentEscrowClosed,
    state::{AgentEscrow, AgentStatus},
};

/// Close an empty escrow: close its (empty) USDC ATA and the data account, reclaiming rent
/// to the owner. Funds must be swept out first (Phase 1 is deposit-only; a withdraw path is
/// Phase 1.5), so the ATA must hold zero USDC.
pub fn close_agent_escrow(ctx: Context<CloseAgentEscrow>) -> Result<()> {
    let escrow = &ctx.accounts.agent_escrow;
    require!(
        ctx.accounts.escrow_ata.amount == 0,
        MolphaError::AgentEscrowNotEmpty
    );
    require!(
        escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );

    // Close the escrow ATA, returning its rent to the owner. The escrow PDA is the ATA
    // authority and signs.
    let owner_key = escrow.owner;
    let authority_key = escrow.authority;
    let bump = [escrow.bump];
    let signer_seeds: &[&[u8]] = &[
        AgentEscrow::SEED_PREFIX,
        owner_key.as_ref(),
        authority_key.as_ref(),
        &bump,
    ];
    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        CloseAccount {
            account: ctx.accounts.escrow_ata.to_account_info(),
            destination: ctx.accounts.owner.to_account_info(),
            authority: ctx.accounts.agent_escrow.to_account_info(),
        },
        &[signer_seeds],
    ))?;

    emit!(AgentEscrowClosed {
        agent: ctx.accounts.agent_escrow.key(),
        owner: ctx.accounts.agent_escrow.owner,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct CloseAgentEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        close = owner,
        constraint = agent_escrow.owner == owner.key() @ MolphaError::UnauthorisedAgentOwner,
    )]
    pub agent_escrow: Account<'info, AgentEscrow>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = agent_escrow,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    pub usdc_mint: Account<'info, token::Mint>,

    pub token_program: Program<'info, Token>,
}
