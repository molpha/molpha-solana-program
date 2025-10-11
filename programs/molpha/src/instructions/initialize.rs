use crate::events::ProtocolInitialized;
use crate::state::ProtocolConfig;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
    let clock = Clock::get()?;

    // Initialize ProtocolConfig
    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.authority = ctx.accounts.authority.key();
    protocol_config.underlying_token = ctx.accounts.underlying_token.key();
    protocol_config.bump = ctx.bumps.protocol_config;

    // Emit event
    emit!(ProtocolInitialized {
        protocol_config: ctx.accounts.protocol_config.key(),
        authority: ctx.accounts.authority.key(),
        initialized_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = ProtocolConfig::SPACE,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// The SPL token mint that will be used for subscription payments
    pub underlying_token: InterfaceAccount<'info, Mint>,

    /// Program-owned associated token account to receive subscription payments
    #[account(
        init,
        payer = authority,
        associated_token::mint = underlying_token,
        associated_token::authority = protocol_config,
    )]
    pub program_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}
