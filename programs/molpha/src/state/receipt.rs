use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const DOMAIN_REQAUTH: &[u8] = b"MOLPHA_REQAUTH_V1";
pub const DOMAIN_GWRCPT: &[u8] = b"MOLPHA_GWRCPT_V1";

/// Consumer authorization fields signed by the requester.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq, InitSpace)]
pub struct RequestAuth {
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

/// `H(RequestAuth)` — domain-separated SHA-256 over the borsh encoding.
pub fn hash_request_auth(request_auth: &RequestAuth) -> [u8; 32] {
    let body = borsh::to_vec(request_auth).expect("RequestAuth serialization is infallible");
    hashv(&[DOMAIN_REQAUTH, &body]).to_bytes()
}

/// `H(RequestAuth)` from the instruction-level `(source_id, signatures_required, timestamp)` pair.
pub fn hash_request_auth_fields(
    source_id: &[u8; 32],
    signatures_required: u8,
    timestamp: u64,
) -> [u8; 32] {
    hash_request_auth(&RequestAuth {
        source_id: *source_id,
        signatures_required,
        timestamp,
    })
}
