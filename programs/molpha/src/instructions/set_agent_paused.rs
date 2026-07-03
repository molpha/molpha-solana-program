use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentStatusChanged,
    state::{AgentEscrow, AgentStatus},
};

/// Owner soft control: Active <-> Paused. Cannot touch a Frozen escrow — only the
/// protocol authority can unfreeze (`set_agent_frozen`).
pub fn set_agent_paused(ctx: Context<SetAgentPaused>, paused: bool) -> Result<()> {
    let escrow = &mut ctx.accounts.agent_escrow;
    require!(
        escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );

    let old_status = escrow.status;
    escrow.status = if paused {
        AgentStatus::Paused
    } else {
        AgentStatus::Active
    };

    emit!(AgentStatusChanged {
        agent: escrow.key(),
        old_status: old_status as u8,
        new_status: escrow.status as u8,
        by: ctx.accounts.owner.key(),
    });

    Ok(())
}

#[derive(Accounts)]
pub struct SetAgentPaused<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        constraint = agent_escrow.owner == owner.key() @ MolphaError::UnauthorisedAgentOwner,
    )]
    pub agent_escrow: Account<'info, AgentEscrow>,
}
