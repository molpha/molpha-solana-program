use anchor_lang::prelude::*;

use crate::{
    events::{RegistryUpdated, RegistryVersionReason},
    state::{ProtocolConfig, Registry, RegistryState, RegistryTransition},
};

pub fn update_registry_policy(
    ctx: Context<UpdateRegistryPolicy>,
    redundancy_buffer: u8,
) -> Result<()> {
    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    // Membership is unchanged; read before the transition bumps `registry_state.next_version`,
    // which would break exit validation on `new_registry`'s version seed.
    let node_count = ctx.accounts.current_registry.load()?.node_count;
    ctx.accounts.registry_state.set_redundancy_buffer(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        redundancy_buffer,
    )?;

    emit!(RegistryUpdated {
        new_version: ctx.accounts.registry_state.current_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::PolicyChanged,
        updated_at: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateRegistryPolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = authority,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    pub system_program: Program<'info, System>,
}
