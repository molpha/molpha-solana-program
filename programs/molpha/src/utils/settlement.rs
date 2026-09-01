use crate::{
    error::{map_attestation_err, MolphaError},
    state::{AggregatedSig, Registry, RegistryState},
    utils::require_fresh_canonical_timestamp,
};
use anchor_lang::prelude::*;
use molpha_verifier::{
    bitmap::{bitmap_is_subset, bitmap_popcount},
    compute_message_hash, derive_selection_bitmap, AttestationPayload,
};

pub struct SettlementAttestationInput<'a> {
    pub source_id: &'a [u8; 32],
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub value: &'a [u8; 32],
    pub agg_sig: &'a AggregatedSig,
}

pub struct ValidatedSettlementAttestation {
    pub credited_count: u32,
    pub attestation_message_hash: [u8; 32],
}

/// Shared optimistic attestation shape checks for subscription and x402 settlement.
pub fn validate_settlement_attestation(
    registry_state: &RegistryState,
    registry: &Registry,
    input: SettlementAttestationInput<'_>,
    now: i64,
    max_receipt_delay_seconds: u64,
) -> Result<ValidatedSettlementAttestation> {
    require!(
        input.registry_version == registry_state.current_version
            && registry.version == input.registry_version,
        MolphaError::InvalidRegistryVersion
    );

    require_fresh_canonical_timestamp(input.canonical_timestamp, now, max_receipt_delay_seconds)?;

    let expected_selection = derive_selection_bitmap(
        input.source_id,
        input.registry_version,
        input.canonical_timestamp,
        u32::from(registry.node_count),
        input.signatures_required,
        registry.redundancy_buffer,
    )
    .map_err(map_attestation_err)?;

    require!(
        bitmap_is_subset(&input.agg_sig.signers_bitmap, &expected_selection),
        MolphaError::SignersNotSubsetOfSelection
    );

    let credited_count = bitmap_popcount(&input.agg_sig.signers_bitmap);
    require!(
        credited_count >= u32::from(input.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        credited_count
            <= u32::from(input.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

    let attestation_message_hash = compute_message_hash(
        &AttestationPayload {
            value: *input.value,
            source_id: *input.source_id,
            signatures_required: input.signatures_required,
            canonical_timestamp: input.canonical_timestamp,
            registry_version: input.registry_version,
        },
        input.agg_sig.signers_bitmap,
    );

    Ok(ValidatedSettlementAttestation {
        credited_count,
        attestation_message_hash,
    })
}
