pub mod dispute_round;
pub mod punish_equivocation;
pub mod punish_unsolicited_signing;

pub use dispute_round::*;
pub use punish_equivocation::*;
pub use punish_unsolicited_signing::*;

use anchor_lang::prelude::*;

use crate::{error::map_data_update_err, error::MolphaError, state::SignedStatement};

/// Verify one signed statement's aggregate Schnorr signature against its resolved signers.
///
/// Shared by both node-punishment offences: `punish_equivocation` calls this once per statement
/// (over each statement's own signer subset), `punish_unsolicited_signing` calls it once (over the
/// full submitted signer set, since the signature itself is what proves the unselected node
/// actually contributed).
pub(crate) fn verify_statement(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: i64,
    statement: &SignedStatement,
    ordered_signers: &[molpha_verifier::SignerXy],
) -> Result<()> {
    let message_hash = molpha_verifier::compute_message_hash(
        &molpha_verifier::DataUpdate {
            source_id: *source_id,
            registry_version,
            value: statement.value.clone(),
            canonical_timestamp,
            signatures_required,
            agg_sig_s: statement.agg_sig_s,
            commitment_addr: statement.commitment_addr,
            signers_bitmap: statement.signers_bitmap,
        },
        signatures_required,
    );
    let is_valid = molpha_verifier::verify_aggregate_over_hash(
        ordered_signers,
        &statement.agg_sig_s,
        &statement.commitment_addr,
        &message_hash,
    )
    .map_err(map_data_update_err)?;
    require!(is_valid, MolphaError::InvalidAggregateSignature);
    Ok(())
}
