use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{
    derive_selection_bitmap, solana::resolve_signers_accounts, Attestation, Bitmap,
};

use crate::error::{map_attestation_err, MolphaError};
use crate::state::{Evidence, EvidenceAccounts};
use crate::utils::slash::{attestation_hash, verify_evidence_signature};

pub const DOMAIN_UNSOLICITED_SIGNING: &[u8] = b"MOLPHA_UNSOLICITED_SIGNING_V1";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UnsolicitedSigningEvidence {
    pub attestation: Attestation,
}

impl Evidence for UnsolicitedSigningEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        let attestation = &self.attestation;

        let signers_bitmap = Bitmap::load(&attestation.signature.signers_bitmap);
        let selection = derive_selection_bitmap(
            &attestation.payload.source_id,
            attestation.payload.canonical_timestamp,
            attestation.payload.signatures_required,
            &ctx.evidence_registry.load()?.view(),
        )
        .map_err(map_attestation_err)?;
        require!(
            !signers_bitmap.is_subset(&selection),
            MolphaError::SignersFullySelected
        );

        let guilty_bitmap = signers_bitmap.subtract(&selection);

        // Resolve the full submitted bitmap; the aggregate is only verifiable over all contributors.
        // `molpha_verifier::verify` would reject this evidence outright, since it enforces the very
        // selection membership the offence breaks.
        let signers = resolve_signers_accounts(
            ctx.remaining,
            ctx.evidence_registry.as_ref(),
            &attestation.signature.signers_bitmap,
        )
        .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
        verify_evidence_signature(attestation, &signers)?;

        Ok((signers_bitmap, guilty_bitmap))
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
