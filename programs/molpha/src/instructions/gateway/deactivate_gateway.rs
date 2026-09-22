use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::GatewayDeactivated,
    state::{Gateway, GatewayStatus, ProtocolConfig},
};

pub fn deactivate_gateway(ctx: Context<DeactivateGateway>) -> Result<()> {
    let gateway = &mut ctx.accounts.gateway;
    let clock = Clock::get()?;

    gateway.status = GatewayStatus::Deactivated;

    emit!(GatewayDeactivated {
        gateway: gateway.key(),
        authority: gateway.authority,
        deactivated_at: clock.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct DeactivateGateway<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        constraint = gateway.status == GatewayStatus::Active @ MolphaError::GatewayNotActive,
    )]
    pub gateway: Account<'info, Gateway>,

    pub system_program: Program<'info, System>,
}
