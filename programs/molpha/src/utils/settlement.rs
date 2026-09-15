use crate::{
    error::{map_attestation_err, MolphaError},
    state::{AggregatedSig, ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};
use anchor_lang::prelude::*;
use molpha_verifier::{
    bitmap::Bitmap,
    compute_message_hash,
    selection::{derive_selection_bitmap, verify_selection},
    AttestationPayload,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundRequest {
    pub source_id: [u8; 32],
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
}

pub struct ValidatedSettlementAttestation {
    pub credited_count: u32,
    pub selection_bitmap: [u8; 32],
    pub attestation_message_hash: [u8; 32],
}

/// Shared optimistic attestation shape checks for subscription and x402 settlement.
pub fn validate_settlement_attestation(
    registry: &Registry,
    config: &ProtocolConfig,
    round_request: &RoundRequest,
    agg_sig: &AggregatedSig,
    value: [u8; 32],
    now: i64,
) -> Result<ValidatedSettlementAttestation> {
    require!(registry.is_active(now), MolphaError::RegistryNotActive);

    require!(
        round_request.signatures_required >= config.min_signers,
        MolphaError::SignaturesBelowMinSigners
    );

    require_fresh_canonical_timestamp(
        round_request.canonical_timestamp,
        now,
        config.max_settlement_delay_seconds,
    )?;

    match verify_selection(
        &round_request.source_id,
        round_request.canonical_timestamp,
        round_request.signatures_required,
        &registry.view(),
        &agg_sig.signers_bitmap,
    ) {
        Ok(true) => {}
        Ok(false) => return Err(error!(MolphaError::SignersNotSubsetOfSelection)),
        Err(err) => return Err(map_attestation_err(err)),
    }

    let credited_count = Bitmap::load(&agg_sig.signers_bitmap).popcount() as u32;
    require!(
        credited_count >= u32::from(round_request.signatures_required),
        MolphaError::QuorumBelowThreshold
    );

    require!(
        credited_count
            <= u32::from(round_request.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

    let attestation_message_hash = compute_message_hash(
        &AttestationPayload {
            value,
            source_id: round_request.source_id,
            signatures_required: round_request.signatures_required,
            canonical_timestamp: round_request.canonical_timestamp,
            registry_version: round_request.registry_version,
        },
        agg_sig.signers_bitmap,
    );

    // Persist the full duty selection, not the credited signer subset (needed for missed-node finalization).
    let selection_bitmap = derive_selection_bitmap(
        &round_request.source_id,
        round_request.canonical_timestamp,
        round_request.signatures_required,
        &registry.view(),
    )
    .map_err(map_attestation_err)?
    .to_bytes();

    Ok(ValidatedSettlementAttestation {
        credited_count,
        selection_bitmap,
        attestation_message_hash,
    })
}
