use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{derive_selection_bitmap, Attestation, Bitmap, CoalitionKey};

use crate::error::{map_attestation_err, MolphaError};
use crate::state::{Evidence, EvidenceAccounts};
use crate::utils::slash::{attestation_hash, verify_evidence_signature};

pub const DOMAIN_UNSOLICITED_SIGNING: &[u8] = b"MOLPHA_UNSOLICITED_SIGNING_V1";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UnsolicitedSigningEvidence {
    pub attestation: Attestation,
    /// Affine sum of the attestation's signer keys, intruders included. A verification input,
    /// not evidence: it is left out of [`Evidence::hash`], so the case PDA derives from the
    /// attestation alone.
    pub coalition_key: CoalitionKey,
}

impl Evidence for UnsolicitedSigningEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<Bitmap> {
        let attestation = &self.attestation;

        let signers_bitmap = Bitmap::load(&attestation.signature.signers_bitmap);
        let selection = derive_selection_bitmap(
            &attestation.payload.source_id,
            attestation.payload.timestamp,
            attestation.payload.signatures_required,
            &ctx.evidence_registry.load()?.selection_view(),
        )
        .map_err(map_attestation_err)?;
        require!(
            !signers_bitmap.is_subset(&selection),
            MolphaError::SignersFullySelected
        );

        let guilty_bitmap = signers_bitmap.subtract(&selection);

        // Verify the aggregate over the full submitted bitmap; it is only verifiable over all
        // contributors. `molpha_verifier::verify` would reject this evidence outright, since it
        // enforces the very selection membership the offence breaks.
        verify_evidence_signature(ctx, attestation, &self.coalition_key)?;

        Ok(guilty_bitmap)
    }

    fn hash(&self) -> [u8; 32] {
        hashv(&[
            DOMAIN_UNSOLICITED_SIGNING,
            attestation_hash(&self.attestation).as_slice(),
        ])
        .to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.attestation.payload.registry_version
    }
}
