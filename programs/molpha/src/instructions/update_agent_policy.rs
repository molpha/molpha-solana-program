use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentPolicyUpdated,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateAgentPolicyArgs {
    pub max_signers: Option<u8>,
    pub epoch_cap: Option<u64>,
    pub expires_at: Option<i64>,
}

pub fn update_agent_policy(
    ctx: Context<UpdateAgentPolicy>,
    args: UpdateAgentPolicyArgs,
) -> Result<()> {
    let escrow = &mut ctx.accounts.agent_escrow;
    require!(
        escrow.status != AgentStatus::Frozen,
        MolphaError::AgentNotActive
    );
    // Policy only applies to owner-provisioned escrows. self-funded escrows (`owner == authority`)
    // carry caps 0 by design; the hot key cannot be rotated here because `authority` is now a
    // PDA seed (rotate by creating a new escrow).
    require!(
        escrow.owner != escrow.authority,
        MolphaError::UnauthorisedAgentOwner
    );

    if let Some(max_signers) = args.max_signers {
        escrow.max_signers = max_signers;
    }
    if let Some(epoch_cap) = args.epoch_cap {
        escrow.epoch_cap = epoch_cap;
    }
    require!(
        escrow.max_signers > 0 && escrow.epoch_cap > 0,
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
        max_signers: escrow.max_signers,
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
