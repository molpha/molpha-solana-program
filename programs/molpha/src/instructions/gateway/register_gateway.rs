use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::GatewayRegistered,
    state::{Gateway, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterGatewayArgs {
    pub ip: [u8; 4],
    pub port: u16,
    pub bond_amount: u64,
}

pub fn register_gateway(ctx: Context<RegisterGateway>, args: RegisterGatewayArgs) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        args.bond_amount >= config.gateway_bond_min,
        MolphaError::InsufficientBond
    );
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidGatewayEndpoint
    );

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.authority_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.gateway_authority.to_account_info(),
            },
        ),
        args.bond_amount,
    )?;

    let now = Clock::get()?.unix_timestamp;

    let gateway = &mut ctx.accounts.gateway;
    gateway.authority = ctx.accounts.gateway_authority.key();
    gateway.ip = args.ip;
    gateway.port = args.port;
    gateway.bond = args.bond_amount;
    gateway.claimable_rewards = 0;
    gateway.slashed = false;
    gateway.registered_at = now;
    gateway.bump = ctx.bumps.gateway;

    emit!(GatewayRegistered {
        authority: gateway.authority,
        ip: gateway.ip,
        port: gateway.port,
        bond: args.bond_amount,
        registered_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct RegisterGateway<'info> {
    #[account(mut)]
    pub gateway_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = gateway_authority,
        space = Gateway::SPACE,
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(
        mut,
        constraint = authority_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = authority_usdc.owner == gateway_authority.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub authority_usdc: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
