use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{RegistryVersionCreated, RegistryVersionReason},
    state::{ProtocolConfig, Registry, RegistryState},
};

pub fn update_registry_policy(
    ctx: Context<UpdateRegistryPolicy>,
    new_registry_version: u32,
    redundancy_buffer: u8,
) -> Result<()> {
    ctx.accounts
        .registry_state
        .require_next_version(new_registry_version)?;

    let previous_version = ctx.accounts.registry_state.current_version;
    let (node_count, previous_buffer) = {
        let current = ctx.accounts.current_registry.load()?;
        let mut next = ctx.accounts.new_registry.load_init()?;
        next.copy_from_previous(&current, new_registry_version, ctx.bumps.new_registry);
        let previous_buffer = next.redundancy_buffer;
        next.redundancy_buffer = redundancy_buffer;
        (next.node_count, previous_buffer)
    };
    require!(
        redundancy_buffer != previous_buffer,
        MolphaError::RegistryPolicyUnchanged
    );

    ctx.accounts.registry_state.current_version = new_registry_version;
    emit!(RegistryVersionCreated {
        previous_version,
        new_version: new_registry_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::PolicyChanged,
        created_at: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(new_registry_version: u32)]
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
        seeds = [Registry::SEED_PREFIX, new_registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    pub system_program: Program<'info, System>,
}
