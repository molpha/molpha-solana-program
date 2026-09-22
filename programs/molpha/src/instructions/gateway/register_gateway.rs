use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::GatewayRegistered,
    state::{Gateway, GatewayStatus, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterGatewayArgs {
    pub authority: Pubkey,
    pub ip: [u8; 4],
    pub port: u16,
}

pub fn register_gateway(ctx: Context<RegisterGateway>, args: RegisterGatewayArgs) -> Result<()> {
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidGatewayEndpoint
    );

    let now = Clock::get()?.unix_timestamp;

    let gateway = &mut ctx.accounts.gateway;
    gateway.authority = args.authority;
    gateway.ip = args.ip;
    gateway.port = args.port;
    gateway.status = GatewayStatus::Active;
    gateway.registered_at = now;
    gateway.bump = ctx.bumps.gateway;

    emit!(GatewayRegistered {
        authority: gateway.authority,
        ip: gateway.ip,
        port: gateway.port,
        registered_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RegisterGatewayArgs)]
pub struct RegisterGateway<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = authority,
        space = Gateway::SPACE,
        seeds = [Gateway::SEED_PREFIX, args.authority.as_ref()],
        bump
    )]
    pub gateway: Account<'info, Gateway>,

    pub system_program: Program<'info, System>,
}
