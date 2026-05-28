use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::GatewayAdded,
    state::{Gateway, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddGatewayArgs {
    pub ip: [u8; 4],
    pub port: u16,
}

pub fn add_gateway(ctx: Context<AddGateway>, args: AddGatewayArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidGatewayEndpoint
    );

    let clock = Clock::get()?;
    let gateway = &mut ctx.accounts.gateway;
    gateway.ip = args.ip;
    gateway.port = args.port;
    gateway.registered_at = clock.unix_timestamp;
    gateway.bump = ctx.bumps.gateway;

    emit!(GatewayAdded {
        ip: gateway.ip,
        port: gateway.port,
        registered_at: gateway.registered_at,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: AddGatewayArgs)]
pub struct AddGateway<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = authority,
        space = Gateway::SPACE,
        seeds = [Gateway::SEED_PREFIX, &args.ip, &args.port.to_le_bytes()],
        bump
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}
