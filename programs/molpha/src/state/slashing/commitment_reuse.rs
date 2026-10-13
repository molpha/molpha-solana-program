use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{Attestation, Bitmap, CoalitionKey};

use crate::error::MolphaError;
use crate::state::{Evidence, EvidenceAccounts};
use crate::utils::slash::{
    attestation_hash, canonical_pair, is_equivocation, require_signers_selected,
    verify_evidence_signature,
};

pub const DOMAIN_COMMITMENT_REUSE: &[u8] = b"MOLPHA_COMMITMENT_REUSE_V1";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CommitmentReuseEvidence {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
    /// Affine sum of `attestation_a`'s signer keys. A verification input, not evidence: it is
    /// left out of [`Evidence::hash`], so the case PDA derives from the attestations alone.
    pub coalition_key_a: CoalitionKey,
    /// Affine sum of `attestation_b`'s signer keys; see `coalition_key_a`.
    pub coalition_key_b: CoalitionKey,
}

impl Evidence for CommitmentReuseEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<Bitmap> {
        let (attestation_a, attestation_b) = (&self.attestation_a, &self.attestation_b);

        require!(
            attestation_a.signature.commitment == attestation_b.signature.commitment,
            MolphaError::NonceCommitmentMismatch
        );

        require!(
            attestation_a.payload.registry_version == attestation_b.payload.registry_version,
            MolphaError::RegistryVersionMismatch
        );

        // The signed message is the payload plus the signer bitmap. Reusing `R` for one message is
        // not an offence, and without this check any published attestation filed twice would slash
        // its own honest signers.
        require!(
            attestation_a.payload != attestation_b.payload
                || attestation_a.signature.signers_bitmap != attestation_b.signature.signers_bitmap,
            MolphaError::NonceReuseSameMessage
        );

        require!(
            !is_equivocation(attestation_a, attestation_b),
            MolphaError::PreferEquivocation
        );

        {
            let evidence_registry = ctx.evidence_registry.load()?;
            let view = evidence_registry.selection_view();
            require_signers_selected(attestation_a, &view)?;
            require_signers_selected(attestation_b, &view)?;
        }

        let guilty_bitmap = Bitmap::load(&attestation_a.signature.signers_bitmap)
            .intersect(&Bitmap::load(&attestation_b.signature.signers_bitmap));
        require!(guilty_bitmap.popcount() > 0, MolphaError::GuiltySetEmpty);

        verify_evidence_signature(ctx, attestation_a, &self.coalition_key_a)?;
        verify_evidence_signature(ctx, attestation_b, &self.coalition_key_b)?;

        Ok(guilty_bitmap)
    }

    fn hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(
            attestation_hash(&self.attestation_a),
            attestation_hash(&self.attestation_b),
        );
        hashv(&[DOMAIN_COMMITMENT_REUSE, lo.as_slice(), hi.as_slice()]).to_bytes()
    }

    fn registry_version(&self) -> u32 {
        self.attestation_a.payload.registry_version
    }
}
