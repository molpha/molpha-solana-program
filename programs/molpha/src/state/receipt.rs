use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::utils::settlement::RoundRequest;

pub const DOMAIN_REQAUTH: &[u8] = b"MOLPHA_REQAUTH_V1";

/// Consumer authorization fields signed by the requester.
///
/// `program_id` and `gateway` scope the authorization: the signature is only redeemable by the
/// gateway the consumer chose, on the deployment they meant to pay. Without them the hash is a
/// bearer token any observer can lift out of the settling transaction and redeem under their own
/// gateway.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq, InitSpace)]
pub struct RequestAuth {
    pub program_id: Pubkey,
    pub gateway: Pubkey,
    pub source_id: [u8; 32],
    pub signatures_required: u8,
    pub timestamp: u64,
}

/// Aggregate Schnorr signature committed at settlement; not verified on the happy path.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AggregatedSig {
    pub agg_sig_s: [u8; 32],
    pub commitment: [u8; 20],
    pub signers_bitmap: [u8; 32],
}

/// `H(RequestAuth)` — domain-separated Keccak256 over the borsh encoding.
pub fn hash_request_auth(request_auth: &RequestAuth) -> [u8; 32] {
    let body = borsh::to_vec(request_auth).expect("RequestAuth serialization is infallible");
    hashv(&[DOMAIN_REQAUTH, &body]).to_bytes()
}

/// `H(RequestAuth)` from the instruction-level `(gateway, source_id, signatures_required,
/// timestamp)` tuple. `program_id` is always this program's own id, so a signature made for one
/// deployment never verifies against another.
pub fn hash_request_auth_fields(
    gateway: &Pubkey,
    round_request: &RoundRequest,
) -> [u8; 32] {
    hash_request_auth(&RequestAuth {
        program_id: crate::ID,
        gateway: *gateway,
        source_id: round_request.source_id,
        signatures_required: round_request.signatures_required,
        timestamp: round_request.canonical_timestamp,
    })
}
