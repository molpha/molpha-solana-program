use anchor_lang::prelude::*;
use molpha_verifier::Attestation;

use crate::{
    error::{map_attestation_accounts_err, map_attestation_err},
    state::Registry,
};

use super::SubmitAttestationArgs;

/// Verify an attestation against any retained historical registry without mutating live state.
pub fn verify_attestation(
    ctx: Context<VerifyAttestation>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let registry = ctx.accounts.registry.load()?;
    require!(
        registry.version == args.registry_version,
        crate::error::MolphaError::InvalidRegistryVersion
    );
    let entries = molpha_verifier::solana::resolve_nodes(ctx.remaining_accounts, ctx.program_id)
        .map_err(map_attestation_accounts_err)?;
    let ordered = molpha_verifier::resolve_registry_signers(
        &registry.nodes,
        registry.node_count,
        &args.signers_bitmap,
        &entries,
    )
    .map_err(map_attestation_err)?;
    let attestation: Attestation = (&args).into();
    molpha_verifier::verify_attestation(
        &attestation,
        u32::from(registry.node_count),
        registry.redundancy_buffer,
        &ordered,
    )
    .map_err(map_attestation_err)
}

#[derive(Accounts)]
#[instruction(args: SubmitAttestationArgs)]
pub struct VerifyAttestation<'info> {
    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,
}
