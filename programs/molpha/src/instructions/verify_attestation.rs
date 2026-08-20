use anchor_lang::prelude::*;
use molpha_verifier::{Attestation, AttestationPayload, SchnorrSignature};

use crate::{
    error::map_attestation_err,
    state::{resolve_registry_signers, Registry},
};

use super::SubmitAttestationArgs;

/// Verify an attestation against any retained historical registry without mutating live state.
pub fn verify_attestation(
    ctx: Context<VerifyAttestation>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let payload: AttestationPayload = (&args).into();
    let sig: SchnorrSignature = (&args).into();
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
    let attestation: Attestation = Attestation{payload, signature: sig};
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
