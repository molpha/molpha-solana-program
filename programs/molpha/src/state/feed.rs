use anchor_lang::prelude::*;

use crate::error::MolphaError;
use molpha_verifier::Attestation;

/// How to read [`Feed::value`], which is always the signed `AttestationPayload::value`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum FeedValueKind {
    /// `value` is the oracle value itself.
    Value,
    /// `value` is `keccak256(raw_value)`; the preimage travelled in the submitting instruction.
    Hash,
}

#[account]
#[derive(InitSpace)]
pub struct Feed {
    pub source_id: [u8; 32],
    pub value: [u8; 32],
    pub value_kind: FeedValueKind,
    pub submitter: Pubkey,
    /// Unix milliseconds. Updates are strictly ordered on it, so two rounds in one second order.
    pub timestamp: u64,
    pub signatures_required: u8,
    pub signers_bitmap: [u8; 32],
    pub registry_version: u32,
    pub bump: u8,
}

impl Feed {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_feed";
    /// Upper bound for a `raw_value` preimage submitted alongside a hashed value.
    pub const MAX_VALUE_LEN: usize = 256;
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        attestation: &Attestation,
        kind: FeedValueKind,
        submitter: Pubkey,
        bump: u8,
    ) -> Result<()> {
        require!(
            attestation.payload.source_id != [0u8; 32],
            MolphaError::InvalidSourceId
        );

        self.source_id = attestation.payload.source_id;
        self.value = attestation.payload.value;
        self.value_kind = kind;
        self.submitter = submitter;
        self.timestamp = attestation.payload.timestamp;
        self.signatures_required = attestation.payload.signatures_required;
        self.registry_version = attestation.payload.registry_version;
        self.signers_bitmap = attestation.signature.signers_bitmap;
        self.bump = bump;
        Ok(())
    }

    pub fn update(&mut self, attestation: &Attestation, kind: FeedValueKind) -> Result<()> {
        require!(
            attestation.payload.source_id == self.source_id,
            MolphaError::InvalidSourceId
        );
        require!(
            attestation.payload.signatures_required == self.signatures_required,
            MolphaError::SignaturesRequiredMismatch
        );
        require!(
            attestation.payload.timestamp > self.timestamp,
            MolphaError::FeedNotNewer
        );
        require!(
            attestation.payload.registry_version >= self.registry_version,
            MolphaError::RegistryVersionMismatch
        );

        self.value = attestation.payload.value;
        self.value_kind = kind;
        self.timestamp = attestation.payload.timestamp;
        self.registry_version = attestation.payload.registry_version;
        self.signers_bitmap = attestation.signature.signers_bitmap;
        Ok(())
    }
}
