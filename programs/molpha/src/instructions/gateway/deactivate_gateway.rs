use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::GatewayDeactivated,
    state::{Gateway, GatewayStatus, ProtocolConfig},
};

pub fn deactivate_gateway(ctx: Context<DeactivateGateway>) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let gateway = &mut ctx.accounts.gateway;
    let clock = Clock::get()?;

    let withdrawable_slot = clock
        .slot
        .checked_add(config.dispute_window_slots)
        .and_then(|slot| slot.checked_add(1))
        .ok_or(MolphaError::WithdrawalArithmeticOverflow)?;

    gateway.status = GatewayStatus::Deactivated;
    gateway.withdrawable_slot = withdrawable_slot;

    emit!(GatewayDeactivated {
        gateway: gateway.key(),
        authority: gateway.authority,
        deactivated_at: clock.unix_timestamp,
        withdrawable_slot,
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

    #[account(mut)]
    pub gateway: Account<'info, Gateway>,

    pub system_program: Program<'info, System>,
}
