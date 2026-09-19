use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{solana::resolve_intersected_signers_accounts, Attestation, Bitmap};

use crate::error::{map_attestation_accounts_err, MolphaError};
use crate::state::{Evidence, EvidenceAccounts};
use crate::utils::slash::{
    attestation_hash, canonical_pair, is_equivocation, verify_evidence_signature,
};

pub const DOMAIN_EQUIVOCATION: &[u8] = b"MOLPHA_EQUIVOCATION_V1";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct EquivocationEvidence {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
}

impl Evidence for EquivocationEvidence {
    fn validate(&self, ctx: &EvidenceAccounts<'_, '_>) -> Result<(Bitmap, Bitmap)> {
        require!(
            is_equivocation(&self.attestation_a, &self.attestation_b),
            MolphaError::NotEquivocation
        );

        let resolution = resolve_intersected_signers_accounts(
            ctx.remaining,
            ctx.evidence_registry.as_ref(),
            &self.attestation_a.signature.signers_bitmap,
            &self.attestation_b.signature.signers_bitmap,
        )
        .map_err(map_attestation_accounts_err)?;

        require!(
            resolution.intersected_bitmap.popcount() > 0,
            MolphaError::GuiltySetEmpty
        );

        verify_evidence_signature(&self.attestation_a, &resolution.signers_a)?;
        verify_evidence_signature(&self.attestation_b, &resolution.signers_b)?;

        Ok((resolution.unioned_bitmap, resolution.intersected_bitmap))
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
