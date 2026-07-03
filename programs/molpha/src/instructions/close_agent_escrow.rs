use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentEscrowClosed,
    state::{AgentEscrow, AgentStatus},
};

/// Close an empty escrow and reclaim rent. Phase 1 is deposit-only, so a funded escrow
/// stays open until the Phase 1.5 withdraw path lands (accepted).
pub fn close_agent_escrow(ctx: Context<CloseAgentEscrow>) -> Result<()> {
    let escrow = &ctx.accounts.agent_escrow;
    require!(escrow.balance == 0, MolphaError::AgentEscrowNotEmpty);
    require!(
        escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );

    emit!(AgentEscrowClosed {
        agent: escrow.key(),
        owner: escrow.owner,
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
}
