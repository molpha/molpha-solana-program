// //! Instruction-facing attestation types.
// //!
// //! Field-for-field (and borsh-byte-for-byte) mirrors of the `molpha_verifier` structs. The verifier
// //! stays framework-agnostic, so its types carry no Anchor `IdlBuild` impl and cannot appear in
// //! instruction arguments directly; these do, and convert at the verifier call sites.

// use anchor_lang::prelude::*;

// /// Signed oracle attestation payload.
// #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
// pub struct AttestationPayload {
//     pub value: [u8; 32],
//     pub source_id: [u8; 32],
//     pub registry_version: u32,
//     pub signatures_required: u8,
//     pub canonical_timestamp: u64,
// }

// /// Aggregate Schnorr signature material.
// #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
// pub struct SchnorrSignature {
//     pub agg_sig_s: [u8; 32],
//     /// Ethereum-style address of the aggregate nonce `R`.
//     pub commitment: [u8; 20],
//     pub signers_bitmap: [u8; 32],
// }

// /// Payload plus the aggregate signature over it.
// #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
// pub struct Attestation {
//     pub payload: AttestationPayload,
//     pub signature: SchnorrSignature,
// }

// impl Attestation {
//     /// Message hash the aggregate signature must verify over.
//     pub fn message_hash(&self) -> [u8; 32] {
//         molpha_verifier::compute_message_hash(&self.payload.into(), self.signature.signers_bitmap)
//     }
// }

// impl From<AttestationPayload> for molpha_verifier::AttestationPayload {
//     fn from(payload: AttestationPayload) -> Self {
//         Self {
//             value: payload.value,
//             source_id: payload.source_id,
//             registry_version: payload.registry_version,
//             signatures_required: payload.signatures_required,
//             canonical_timestamp: payload.canonical_timestamp,
//         }
//     }
// }

// impl From<SchnorrSignature> for molpha_verifier::SchnorrSignature {
//     fn from(signature: SchnorrSignature) -> Self {
//         Self {
//             agg_sig_s: signature.agg_sig_s,
//             commitment: signature.commitment,
//             signers_bitmap: signature.signers_bitmap,
//         }
//     }
// }

// impl From<Attestation> for molpha_verifier::Attestation {
//     fn from(attestation: Attestation) -> Self {
//         Self {
//             payload: attestation.payload.into(),
//             signature: attestation.signature.into(),
//         }
//     }
// }
