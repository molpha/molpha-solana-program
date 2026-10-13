use anchor_lang::prelude::*;

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    state::Registry,
};
use molpha_verifier::{solana::verify_attestation, Attestation, Bitmap, CoalitionKey};

/// Registry-version liveness at a point in time: the current version, or a superseded one whose
/// grace had not expired at `at`.
pub(crate) fn require_registry_active_at(registry: &Registry, at: i64) -> Result<()> {
    require!(registry.is_active(at), MolphaError::RegistryNotActive);
    Ok(())
}

/// Checks shared by every instruction that accepts an attestation: threshold floor, signer count
/// bounds, and the aggregate signature itself. Signer keys come from the registry snapshot the
/// attestation names, so no signer accounts are passed. Callers add their own time and
/// registry-liveness checks.
pub(crate) fn verify_attestation_core<'info>(
    registry_acc: &AccountLoader<'info, Registry>,
    min_signers: u8,
    attestation: &Attestation,
    coalition_key: &CoalitionKey,
) -> Result<()> {
    let redundancy_buffer = registry_acc.load()?.redundancy_buffer;

    require!(
        attestation.payload.signatures_required >= min_signers,
        MolphaError::SignaturesBelowMinSigners
    );

    let signers = Bitmap::load(&attestation.signature.signers_bitmap);
    let signer_count = signers.popcount();
    require!(
        signer_count >= u32::from(attestation.payload.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        signer_count
            <= u32::from(attestation.payload.signatures_required) + u32::from(redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

    verify_attestation(
        attestation,
        &registry_acc.to_account_info(),
        Some(coalition_key),
    )
    .map_err(map_attestation_accounts_err)?;

    Ok(())
}
