use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

use crate::{
    error::MolphaError,
    events::AgentEscrowCreated,
    state::{AgentEscrow, AgentStatus, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateAgentEscrowArgs {
    /// Hot runtime key that signs `AgentRequestAuth` messages. Also a seed component —
    /// distinct from `owner` (cold/hot split), which is what marks this as owner-provisioned.
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
    // Owner-provisioned escrows require a cold/hot split. `owner == authority` is reserved for
    // self-funded escrows (lazily created at settlement with caps 0); enforcing the split
    // here keeps the `owner == authority` settlement discriminator unambiguous.
    require!(
        args.authority != Pubkey::default() && args.authority != ctx.accounts.owner.key(),
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
    escrow.status = AgentStatus::Active;
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
            args.authority.as_ref(),
        ],
        bump
    )]
    pub agent_escrow: Account<'info, AgentEscrow>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Account<'info, Mint>,

    /// The escrow's own USDC ATA — real custody. Owner pays rent.
    #[account(
        init,
        payer = owner,
        associated_token::mint = usdc_mint,
        associated_token::authority = agent_escrow,
        associated_token::token_program = token_program,
    )]
    pub escrow_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
