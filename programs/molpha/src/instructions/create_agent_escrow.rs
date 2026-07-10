use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::AgentEscrowCreated,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateAgentEscrowArgs {
    pub agent_index: u64,
    /// Hot runtime key that signs `AgentRequestAuth` messages.
    pub authority: Pubkey,
    pub max_signers: u8,
    pub epoch_cap: u64,
    /// 0 = never expires.
    pub expires_at: i64,
}

pub fn create_agent_escrow(
    ctx: Context<CreateAgentEscrow>,
    args: CreateAgentEscrowArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        args.max_signers > 0 && args.epoch_cap > 0,
        MolphaError::InvalidAgentCaps
    );
    require!(
        args.authority != Pubkey::default(),
        MolphaError::InvalidAgentCaps
    );
    let now = Clock::get()?.unix_timestamp;
    require!(
        args.expires_at == 0 || args.expires_at > now,
        MolphaError::AgentExpired
    );

    let escrow = &mut ctx.accounts.agent_escrow;
    escrow.owner = ctx.accounts.owner.key();
    escrow.authority = args.authority;
    escrow.agent_index = args.agent_index;
    escrow.status = AgentStatus::Active;
    escrow.balance = 0;
    escrow.spent_total = 0;
    escrow.max_signers = args.max_signers;
    escrow.epoch_cap = args.epoch_cap;
    escrow.epoch_spent = 0;
    escrow.epoch_id = now / config.agent_epoch_len_secs;
    escrow.expires_at = args.expires_at;
    escrow.last_request_nonce = 0;
    escrow.bump = ctx.bumps.agent_escrow;

    emit!(AgentEscrowCreated {
        agent: escrow.key(),
        owner: escrow.owner,
        authority: escrow.authority,
        agent_index: args.agent_index,
        max_signers: args.max_signers,
        epoch_cap: args.epoch_cap,
        expires_at: args.expires_at,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: CreateAgentEscrowArgs)]
pub struct CreateAgentEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = owner,
        space = AgentEscrow::SPACE,
        seeds = [
            AgentEscrow::SEED_PREFIX,
            owner.key().as_ref(),
            args.agent_index.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub agent_escrow: Account<'info, AgentEscrow>,

    pub system_program: Program<'info, System>,
}
