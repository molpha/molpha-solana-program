use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::{attestation_selection, Attestation, Bitmap, CoalitionKey};

use crate::error::{map_attestation_err, MolphaError};
use crate::state::{
    selection_source_id, Evidence, EvidenceAccounts, InconclusiveLink, RegistryRef,
};
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

        // Provider bits are not unsolicited: only node bits are compared with the selection, which
        // is the committee of the round's node quorum (one less node when the provider signed).
        let all_signers = Bitmap::load(&attestation.signature.signers_bitmap);
        let signers_bitmap = all_signers.node_bits();
        let registry_account = ctx.evidence_registry.to_account_info();
        let registry = RegistryRef::load(&registry_account)?;
        // The committee as the round drew it: under S for an INCONCLUSIVE attestation whose link
        // exists, so S's honest committee is never read as unsolicited for signing I(S). The link's
        // address is the last remaining account, after the guilty nodes, and it is required: a filing
        // that leaves it out cannot have S's committee drawn under I(S).
        let link = ctx
            .remaining
            .last()
            .ok_or(error!(MolphaError::InvalidInconclusiveLink))?;
        let source_id = &attestation.payload.source_id;
        let (expected, _) = Pubkey::find_program_address(
            &[InconclusiveLink::SEED_PREFIX, source_id.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(link.key(), expected, MolphaError::InvalidInconclusiveLink);
        let selection_id = selection_source_id(link, source_id)?;
        let selection = attestation_selection(
            &selection_id,
            attestation.payload.timestamp,
            attestation.payload.signatures_required,
            &all_signers,
            &registry.view(),
        )
        .map_err(map_attestation_err)?;
        drop(registry);
        require!(
            !signers_bitmap.is_subset(&selection),
            MolphaError::SignersFullySelected
        );

        let guilty_bitmap = signers_bitmap.subtract(&selection);

        // Verify the aggregate over the full submitted bitmap, provider bits included; it is only
        // verifiable over all contributors. The signature-only path is used because the evidence needs no quorum.
        // Consumers accept such an attestation (`molpha_verifier::verify` does not look at the
        // selection), so this offence is the only thing standing against an unsolicited signer.
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
