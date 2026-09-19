use crate::{
    error::{map_attestation_err, MolphaError},
    state::{ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};
use anchor_lang::prelude::*;
use molpha_verifier::Attestation;
use molpha_verifier::{bitmap::Bitmap, selection::derive_selection_bitmap};

pub struct ValidatedSettlementAttestation {
    pub credited_count: u32,
    pub selection_bitmap: [u8; 32],
    pub attestation_message_hash: [u8; 32],
}

/// Shared optimistic attestation shape checks for subscription and x402 settlement.
pub fn validate_settlement_attestation(
    registry: &Registry,
    config: &ProtocolConfig,
    attestation: &Attestation,
    now: i64,
) -> Result<ValidatedSettlementAttestation> {
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

    // Derived once: it bounds the signers here and is persisted whole (not just the credited
    // subset) for missed-node finalization.
    let selection = derive_selection_bitmap(
        &attestation.payload.source_id,
        attestation.payload.canonical_timestamp,
        attestation.payload.signatures_required,
        &registry.view(),
    )
    .map_err(map_attestation_err)?;

    let signers = Bitmap::load(&attestation.signature.signers_bitmap);
    require!(
        signers.is_subset(&selection),
        MolphaError::SignersNotSubsetOfSelection
    );

    let credited_count = signers.popcount();
    require!(
        credited_count >= u32::from(attestation.payload.signatures_required),
        MolphaError::QuorumBelowThreshold
    );

    require!(
        credited_count
            <= u32::from(attestation.payload.signatures_required)
                + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

    Ok(ValidatedSettlementAttestation {
        credited_count,
        selection_bitmap: selection.to_bytes(),
        attestation_message_hash: attestation.message_hash(),
    })
}
