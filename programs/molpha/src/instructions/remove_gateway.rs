use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::GatewayRemoved,
    state::{Gateway, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RemoveGatewayArgs {
    pub ip: [u8; 4],
    pub port: u16,
}

pub fn remove_gateway(ctx: Context<RemoveGateway>, _args: RemoveGatewayArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );

    emit!(GatewayRemoved {
        ip: ctx.accounts.gateway.ip,
        port: ctx.accounts.gateway.port,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RemoveGatewayArgs)]
pub struct RemoveGateway<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        close = authority,
        seeds = [Gateway::SEED_PREFIX, &args.ip, &args.port.to_le_bytes()],
        bump = gateway.bump
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(mut)]
    pub authority: Signer<'info>,
}
