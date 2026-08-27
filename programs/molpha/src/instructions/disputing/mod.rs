pub mod dispute_round;
pub mod punish_equivocation;
pub mod punish_nonce_reuse;
pub mod punish_unsolicited_signing;

pub use dispute_round::*;
pub use punish_equivocation::*;
pub use punish_nonce_reuse::*;
pub use punish_unsolicited_signing::*;

use anchor_lang::prelude::*;

use crate::{error::map_attestation_err, error::MolphaError, state::SignedStatement};

/// Message hash a statement aggregate must verify over.
pub(crate) fn statement_message_hash(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: u64,
    statement: &SignedStatement,
) -> [u8; 32] {
    molpha_verifier::compute_message_hash(
        &molpha_verifier::AttestationPayload {
            source_id: *source_id,
            registry_version,
            value: statement.value.clone(),
            canonical_timestamp: canonical_timestamp as u64,
            signatures_required,
        },
        statement.signers_bitmap,
    )
}

/// Verify one signed statement against its resolved signers.
pub(crate) fn verify_statement(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: u64,
    statement: &SignedStatement,
    ordered_signers: &[molpha_verifier::SignerXy],
) -> Result<()> {
    let message_hash = statement_message_hash(
        source_id,
        registry_version,
        signatures_required,
        canonical_timestamp,
        statement,
    );
    let is_valid = molpha_verifier::verify_aggregate_over_hash(
        ordered_signers,
        &statement.agg_sig_s,
        &statement.commitment_addr,
        &message_hash,
    )
    .map_err(map_attestation_err)?;
    require!(is_valid, MolphaError::InvalidAggregateSignature);
    Ok(())
}
