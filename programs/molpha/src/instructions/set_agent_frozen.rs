use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentStatusChanged,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

/// Protocol-level safety control (abuse, compromised agent, fraud investigation).
///
/// Unfreezing restores to `Paused`, not `Active` — the owner must explicitly resume via
/// `set_agent_paused(false)` after reviewing the incident (safe default).
pub fn set_agent_frozen(ctx: Context<SetAgentFrozen>, frozen: bool) -> Result<()> {
    let escrow = &mut ctx.accounts.agent_escrow;
    let old_status = escrow.status;
    escrow.status = if frozen {
        AgentStatus::Frozen
    } else {
        AgentStatus::Paused
    };

    emit!(AgentStatusChanged {
        agent: escrow.key(),
        old_status: old_status as u8,
        new_status: escrow.status as u8,
        by: ctx.accounts.authority.key(),
    });

    Ok(())
}

#[derive(Accounts)]
pub struct SetAgentFrozen<'info> {
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
        constraint = protocol_config.authority == authority.key() @ MolphaError::UnauthorisedAuthority,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub agent_escrow: Account<'info, AgentEscrow>,
}
