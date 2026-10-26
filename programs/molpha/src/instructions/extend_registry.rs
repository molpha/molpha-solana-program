//! Permissionless: allocate the next registry version's account ahead of the instruction that
//! publishes it.

use anchor_lang::prelude::*;

use crate::{
    events::RegistryExtended,
    state::{Registry, RegistryState, MAX_REGISTRY_NODES},
    utils::registry_account::RegistryAllocation,
};

/// A registry snapshot is sized to its membership, and one instruction can create or grow an
/// account by at most 10 KiB. That covers a snapshot of up to 106 members, so smaller registries
/// never need this. For a larger one, call it once per 10 KiB the next version is short of, in the
/// same transaction as the instruction that publishes that version, which adds the last 10 KiB
/// itself:
///
/// - 107 to 213 members: one call;
/// - 214 to 256 members: two calls.
///
/// Each call grows the account toward the largest size the next version could need: one member
/// more than the current version. The publishing instruction then sizes it to the membership it
/// ends with and returns the rent that turns out not to be needed to its own payer. Called in the
/// publishing transaction, that is the same payer, who ends up paying for the snapshot and no
/// more. Called by someone else, it is a gift to whoever publishes.
///
/// Anyone may call it, and calling it too often or too early is harmless: the account only ever
/// belongs to the next version, it is a no-op once the account is large enough, and the publishing
/// instruction accepts whatever has been allocated.
pub fn extend_registry(ctx: Context<ExtendRegistry>) -> Result<()> {
    let version = ctx.accounts.registry_state.next_version;
    let members = usize::from(ctx.accounts.current_registry.load()?.node_count) + 1;
    let target = Registry::space(members.min(MAX_REGISTRY_NODES));

    let data_len = RegistryAllocation {
        account: &ctx.accounts.new_registry,
        payer: &ctx.accounts.payer,
        system_program: &ctx.accounts.system_program,
        version,
        bump: ctx.bumps.new_registry,
    }
    .extend_toward(target)?;

    emit!(RegistryExtended {
        version,
        data_len: data_len as u32,
        target_len: target as u32,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ExtendRegistry<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    /// CHECK: the next registry version's PDA, not written until that version is published.
    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
