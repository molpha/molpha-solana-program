use anchor_lang::prelude::*;
use molpha_verifier::{Attestation, AttestationPayload};
use solana_keccak_hasher::hashv;

pub const DOMAIN_TICKET: &[u8] = b"MOLPHA_TICKET_V1";
pub const DOMAIN_TICKET_ID: &[u8] = b"MOLPHA_TICKET_ID_V1";
pub const DOMAIN_DISPATCH: &[u8] = b"MOLPHA_DISPATCH_V1";

/// Existence marker for a submitted ticket; blocks resubmitting the same round.
///
/// PDA: `[SEED_PREFIX, ticket_id]`. Once the round's epoch stops accepting tickets the round can
/// never be submitted again, so the marker becomes closable and returns its rent.
#[account]
#[derive(InitSpace)]
pub struct TicketMarker {
    /// The round's epoch ticket deadline (`end + ticket_grace`).
    pub closable_after: i64,
    pub submitter: Pubkey,
    pub bump: u8,
}

impl TicketMarker {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_ticket";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}

/// Round identity: one ticket per `(source_id, signatures_required, canonical_timestamp)`.
///
/// Deliberately excludes the value, bitmap, signature and registry version, so alternative
/// aggregates of one round (other signer subsets, re-signed nonces) cannot count twice.
pub fn ticket_id(payload: &AttestationPayload) -> [u8; 32] {
    hashv(&[
        DOMAIN_TICKET_ID,
        payload.source_id.as_ref(),
        &[payload.signatures_required],
        &payload.canonical_timestamp.to_be_bytes(),
    ])
    .to_bytes()
}

/// `keccak("MOLPHA_TICKET_V1" ‖ m ‖ s ‖ commitment)`: covers the aggregate signature, so nobody
/// without the signers' keys can predict whether a round becomes a ticket.
pub fn ticket_hash(attestation: &Attestation) -> [u8; 32] {
    hashv(&[
        DOMAIN_TICKET,
        attestation.message_hash().as_ref(),
        attestation.signature.agg_sig_s.as_ref(),
        attestation.signature.commitment.as_ref(),
    ])
    .to_bytes()
}

/// Message a gateway signs with its authority key when it dispatches a round, before any node
/// signs. A ticket must carry it, which binds tickets to rounds a registered gateway admitted.
pub fn dispatch_message(program_id: &Pubkey, payload: &AttestationPayload) -> [u8; 32] {
    hashv(&[
        DOMAIN_DISPATCH,
        program_id.as_ref(),
        payload.source_id.as_ref(),
        &[payload.signatures_required],
        &payload.registry_version.to_be_bytes(),
        &payload.canonical_timestamp.to_be_bytes(),
    ])
    .to_bytes()
}
