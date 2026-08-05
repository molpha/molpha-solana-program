use anchor_lang::prelude::*;
use molpha_verifier::DataUpdate;

use crate::{
    error::map_data_update_err,
    state::{resolve_registry_signers, Registry},
};

use super::SubmitDataUpdateArgs;

/// Verify a data update against any retained historical registry without mutating live state.
pub fn verify_data_update(
    ctx: Context<VerifyDataUpdate>,
    args: SubmitDataUpdateArgs,
) -> Result<()> {
    let payload: DataUpdate = (&args).into();
    let registry = ctx.accounts.registry.load()?;
    require!(
        registry.version == args.registry_version,
        crate::error::MolphaError::InvalidRegistryVersion
    );
    let ordered = resolve_registry_signers(
        ctx.remaining_accounts,
        ctx.program_id,
        &registry,
        &args.signers_bitmap,
    )?;
    molpha_verifier::verify_data_update(
        &payload,
        u32::from(registry.node_count),
        registry.redundancy_buffer,
        &ordered,
    )
    .map_err(map_data_update_err)
}

#[derive(Accounts)]
#[instruction(args: SubmitDataUpdateArgs)]
pub struct VerifyDataUpdate<'info> {
    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,
}
