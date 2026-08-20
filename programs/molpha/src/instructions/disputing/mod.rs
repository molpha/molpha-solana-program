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

/// The Molpha message hash a statement's aggregate signature must be valid over.
///
/// Split out of [`verify_statement`] because `punish_nonce_reuse` needs to compare two
/// statements' message hashes (its "distinct signed messages" predicate) before it is worth
/// paying for the two `secp256k1_recover` syscalls signature verification costs.
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

/// Verify one signed statement's aggregate Schnorr signature against its resolved signers.
///
/// Shared by all three node-punishment offences: `punish_equivocation` and `punish_nonce_reuse`
/// each call this once per statement (over each statement's own signer subset),
/// `punish_unsolicited_signing` calls it once (over the full submitted signer set, since the
/// signature itself is what proves the unselected node actually contributed).
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
