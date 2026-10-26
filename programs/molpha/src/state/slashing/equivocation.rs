use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{Attestation, Bitmap, CoalitionKey};

use crate::error::MolphaError;
use crate::state::{Evidence, EvidenceAccounts};
use crate::utils::slash::{
    attestation_hash, canonical_pair, is_equivocation, verify_evidence_signature,
};

pub const DOMAIN_EQUIVOCATION: &[u8] = b"MOLPHA_EQUIVOCATION_V1";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct EquivocationEvidence {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
    /// Affine sum of `attestation_a`'s signer keys. A verification input, not evidence: it is
    /// left out of [`Evidence::hash`], so the case PDA derives from the attestations alone.
    pub coalition_key_a: CoalitionKey,
    /// Affine sum of `attestation_b`'s signer keys; see `coalition_key_a`.
    pub coalition_key_b: CoalitionKey,
}

impl Evidence for EquivocationEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<Bitmap> {
        require!(
            is_equivocation(&self.attestation_a, &self.attestation_b),
            MolphaError::NotEquivocation
        );

        // Both attestations sign the same duty, so a node in both bitmaps signed both values.
        let guilty_bitmap = Bitmap::load(&self.attestation_a.signature.signers_bitmap)
            .intersect(&Bitmap::load(&self.attestation_b.signature.signers_bitmap));
        require!(guilty_bitmap.popcount() > 0, MolphaError::GuiltySetEmpty);

        verify_evidence_signature(ctx, &self.attestation_a, &self.coalition_key_a)?;
        verify_evidence_signature(ctx, &self.attestation_b, &self.coalition_key_b)?;

        Ok(guilty_bitmap)
    }

    fn hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(
            attestation_hash(&self.attestation_a),
            attestation_hash(&self.attestation_b),
        );
        hashv(&[DOMAIN_EQUIVOCATION, lo.as_slice(), hi.as_slice()]).to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.attestation_a.payload.registry_version
    }
}
