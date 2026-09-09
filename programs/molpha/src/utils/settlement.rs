use crate::{
    error::{map_attestation_err, MolphaError},
    state::{AggregatedSig, ProtocolConfig, Registry},
    utils::require_fresh_canonical_timestamp,
};
use anchor_lang::prelude::*;
use molpha_verifier::{
    bitmap::{bitmap_is_subset, bitmap_popcount},
    compute_message_hash, derive_selection_bitmap, AttestationPayload,
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

    let expected_selection = derive_selection_bitmap(
        &round_request.source_id,
        round_request.registry_version,
        round_request.canonical_timestamp,
        u32::from(registry.node_count),
        round_request.signatures_required,
        registry.redundancy_buffer,
    )
    .map_err(map_attestation_err)?;

    require!(
        bitmap_is_subset(&agg_sig.signers_bitmap, &expected_selection),
        MolphaError::SignersNotSubsetOfSelection
    );

    let credited_count = bitmap_popcount(&agg_sig.signers_bitmap);
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

    Ok(ValidatedSettlementAttestation {
        credited_count,
        selection_bitmap: expected_selection,
        attestation_message_hash,
    })
}
