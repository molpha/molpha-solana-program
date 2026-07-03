use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentPolicyUpdated,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateAgentPolicyArgs {
    /// Rotate the hot runtime key.
    pub authority: Option<Pubkey>,
    pub per_request_cap: Option<u64>,
    pub epoch_cap: Option<u64>,
    pub expires_at: Option<i64>,
}

pub fn update_agent_policy(
    ctx: Context<UpdateAgentPolicy>,
    args: UpdateAgentPolicyArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let escrow = &mut ctx.accounts.agent_escrow;
    require!(
        escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );

    if let Some(authority) = args.authority {
        require!(authority != Pubkey::default(), MolphaError::InvalidAgentCaps);
        escrow.authority = authority;
    }
    if let Some(per_request_cap) = args.per_request_cap {
        escrow.per_request_cap = per_request_cap;
    }
    if let Some(epoch_cap) = args.epoch_cap {
        escrow.epoch_cap = epoch_cap;
    }
    require!(
        escrow.per_request_cap > 0
            && escrow.per_request_cap <= config.max_agent_per_request_cap
            && escrow.epoch_cap >= escrow.per_request_cap
            && escrow.epoch_cap <= config.max_agent_epoch_cap,
        MolphaError::InvalidAgentCaps
    );
    if let Some(expires_at) = args.expires_at {
        let now = Clock::get()?.unix_timestamp;
        require!(
            expires_at == 0 || expires_at > now,
            MolphaError::AgentExpired
        );
        escrow.expires_at = expires_at;
    }

    emit!(AgentPolicyUpdated {
        agent: escrow.key(),
        authority: escrow.authority,
        per_request_cap: escrow.per_request_cap,
        epoch_cap: escrow.epoch_cap,
        expires_at: escrow.expires_at,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateAgentPolicy<'info> {
    pub owner: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        constraint = agent_escrow.owner == owner.key() @ MolphaError::UnauthorisedAgentOwner,
    )]
    pub agent_escrow: Account<'info, AgentEscrow>,
}
