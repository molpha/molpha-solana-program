use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{RegistryUpdated, RegistryVersionReason},
    state::{ProtocolConfig, Registry, RegistryState},
};

pub fn update_registry_policy(
    ctx: Context<UpdateRegistryPolicy>,
    redundancy_buffer: u8,
) -> Result<()> {
    require!(
        redundancy_buffer > 0 && redundancy_buffer <= 8,
        MolphaError::InvalidRedundancyBuffer
    );

    let current = ctx.accounts.current_registry.load()?;
    require!(
        redundancy_buffer != current.redundancy_buffer,
        MolphaError::RegistryPolicyUnchanged
    );

    let mut new_registry = ctx.accounts.new_registry.load_init()?;
    new_registry.copy_from_previous(&current, ctx.bumps.new_registry);
    new_registry.redundancy_buffer = redundancy_buffer;

    ctx.accounts.registry_state.advance_version();

    emit!(RegistryUpdated {
        new_version: ctx.accounts.registry_state.next_version,
        node_count: new_registry.node_count,
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
