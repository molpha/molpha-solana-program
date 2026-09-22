use anchor_lang::prelude::*;

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    state::{ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};
use molpha_verifier::{solana::verify_attestation, Attestation, Bitmap};

pub(crate) fn validate_attestation<'info>(
    node_accounts: &'info [AccountInfo<'info>],
    registry_acc: &AccountLoader<'info, Registry>,
    config: &ProtocolConfig,
    attestation: &Attestation,
    now: i64,
) -> Result<()> {
    let registry = registry_acc.load()?;

    require!(registry.is_active(now), MolphaError::RegistryNotActive);
    require!(
        attestation.payload.signatures_required >= config.min_signers,
        MolphaError::SignaturesBelowMinSigners
    );

    require_fresh_canonical_timestamp(
        attestation.payload.canonical_timestamp,
        now,
        config.max_settlement_delay_seconds,
    )?;

    let signers = Bitmap::load(&attestation.signature.signers_bitmap);
    let signer_count = signers.popcount();
    require!(
        signer_count >= u32::from(attestation.payload.signatures_required),
        MolphaError::QuorumBelowThreshold
    );

    require!(
        signer_count
            <= u32::from(attestation.payload.signatures_required)
                + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );
    require!(
        node_accounts.len() >= signer_count as usize,
        MolphaError::MissingSignerAccount
    );

    // Settlement also receives the missed selected nodes after the signer accounts so it can
    // update liveliness in the same instruction. The verifier expects exactly one account per
    // signer, so only pass the signer prefix here.
    verify_attestation(
        attestation,
        &registry_acc.to_account_info(),
        &node_accounts[..signer_count as usize],
    )
    .map_err(map_attestation_accounts_err)?;

    Ok(())
}
