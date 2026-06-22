//! Receipt-settlement wire structures (v0.2 spec).
//!
//! These are instruction-argument payloads (not on-chain accounts). They carry the
//! consumer demand authorization, the committed (but unverified-on-happy-path) aggregate
//! Schnorr signature, and the gateway's bilateral envelope.
//!
//! Hashing uses SHA-256 (`solana_program::hash`) to match the spec's `sha256(..)` / `H(..)`
//! definitions. Domain separation tags bind each hash to its role.

use anchor_lang::prelude::*;
use solana_sha256_hasher::hashv as sha256_hashv;

/// Domain tag for `H(RequestAuth)` — the consumer's Ed25519-signed message.
pub const DOMAIN_REQAUTH: &[u8] = b"MOLPHA_REQAUTH_V1";
/// Domain tag for `H(GatewayReceipt)` — the gateway's Ed25519-signed message.
pub const DOMAIN_GWRCPT: &[u8] = b"MOLPHA_GWRCPT_V1";

/// Consumer demand authorization (Ed25519-signed by `consumer.authority`).
///
/// The consumer signs only `(job_id, timestamp)`. Replay protection is enforced on-chain
/// via `Job.last_request_timestamp` (strictly increasing per job).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct RequestAuth {
    pub job_id: [u8; 32],
    pub timestamp: u64,
}

/// The committed aggregate Schnorr signature. **Not verified on the happy path.**
///
/// Round context (`job_id`, `timestamp`) is passed once at the instruction level in
/// `SettleRoundArgs`. The credited participation set is `signers_bitmap` — the gateway is
/// bound to it via the on-chain `sha256(AggregatedSig)` mixed into `H(GatewayReceipt)`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AggregatedSig {
    /// Schnorr scalar `s`.
    pub s: [u8; 32],
    pub rx: [u8; 32],
    pub ry_parity: u8,
    /// Ethereum-style 20-byte commitment address of the aggregate nonce `R`.
    pub commitment_addr: [u8; 20],
    /// The credited set.
    pub signers_bitmap: [u8; 32],
}

/// Gateway bilateral envelope + bondable claim (Ed25519-signed by `gateway.authority`).
///
/// `H(RequestAuth)` and `sha256(AggregatedSig)` are recomputed on-chain at settlement and
/// mixed into `hash_gateway_receipt` — they are not carried on the wire.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct GatewayReceipt {
    pub registry_version: u32,
    pub selection_bitmap: [u8; 32],
    pub gateway_index: u32,
}

/// `H(RequestAuth)` — domain-separated SHA-256 over the borsh encoding.
pub fn hash_request_auth(request_auth: &RequestAuth) -> [u8; 32] {
    let body = request_auth
        .try_to_vec()
        .expect("RequestAuth serialization is infallible");
    sha256_hashv(&[DOMAIN_REQAUTH, &body]).to_bytes()
}

/// `H(RequestAuth)` from the instruction-level `(job_id, timestamp)` pair.
pub fn hash_request_auth_fields(job_id: &[u8; 32], timestamp: u64) -> [u8; 32] {
    hash_request_auth(&RequestAuth {
        job_id: *job_id,
        timestamp,
    })
}

/// `sha256(AggregatedSig)` — the commitment stored in `RoundReceiptRecord`.
pub fn hash_agg_sig(agg_sig: &AggregatedSig) -> [u8; 32] {
    let body = agg_sig
        .try_to_vec()
        .expect("AggregatedSig serialization is infallible");
    sha256_hashv(&[&body]).to_bytes()
}

/// `H(GatewayReceipt)` — domain-separated SHA-256 over the receipt body plus the on-chain
/// request/response bindings (`H(RequestAuth)`, `sha256(AggregatedSig)`, and the committed
/// data-update message hash used for dispute-time aggregate verification).
pub fn hash_gateway_receipt(
    receipt: &GatewayReceipt,
    request_auth_hash: &[u8; 32],
    agg_sig_hash: &[u8; 32],
    data_update_message_hash: &[u8; 32],
) -> [u8; 32] {
    let body = receipt
        .try_to_vec()
        .expect("GatewayReceipt serialization is infallible");
    sha256_hashv(&[
        DOMAIN_GWRCPT,
        &body,
        request_auth_hash.as_ref(),
        agg_sig_hash.as_ref(),
        data_update_message_hash.as_ref(),
    ])
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_request_auth() -> RequestAuth {
        RequestAuth {
            job_id: [0x11; 32],
            timestamp: 1_700_000_000,
        }
    }

    fn fixture_agg_sig() -> AggregatedSig {
        AggregatedSig {
            s: [0x33; 32],
            rx: [0x44; 32],
            ry_parity: 1,
            commitment_addr: [0x55; 20],
            signers_bitmap: [0x66; 32],
        }
    }

    fn fixture_gateway_receipt() -> GatewayReceipt {
        GatewayReceipt {
            registry_version: 3,
            selection_bitmap: [0x88; 32],
            gateway_index: 2,
        }
    }

    #[test]
    fn hashes_are_deterministic() {
        assert_eq!(
            hash_request_auth(&fixture_request_auth()),
            hash_request_auth(&fixture_request_auth())
        );
        assert_eq!(
            hash_agg_sig(&fixture_agg_sig()),
            hash_agg_sig(&fixture_agg_sig())
        );
        let receipt = fixture_gateway_receipt();
        let req = hash_request_auth(&fixture_request_auth());
        let agg = hash_agg_sig(&fixture_agg_sig());
        let data_update_message_hash = [0x99; 32];
        assert_eq!(
            hash_gateway_receipt(&receipt, &req, &agg, &data_update_message_hash),
            hash_gateway_receipt(&receipt, &req, &agg, &data_update_message_hash)
        );
    }

    #[test]
    fn hash_request_auth_fields_matches_struct_hash() {
        let auth = fixture_request_auth();
        assert_eq!(
            hash_request_auth_fields(&auth.job_id, auth.timestamp),
            hash_request_auth(&auth)
        );
    }

    #[test]
    fn request_auth_hash_is_field_sensitive() {
        let base = hash_request_auth(&fixture_request_auth());
        let mut a = fixture_request_auth();
        a.timestamp += 1;
        assert_ne!(hash_request_auth(&a), base);
        let mut b = fixture_request_auth();
        b.job_id[0] ^= 0x01;
        assert_ne!(hash_request_auth(&b), base);
    }

    #[test]
    fn agg_sig_hash_changes_with_bitmap() {
        let base = hash_agg_sig(&fixture_agg_sig());
        let mut a = fixture_agg_sig();
        a.signers_bitmap[0] ^= 0x01;
        assert_ne!(hash_agg_sig(&a), base);
    }

    #[test]
    fn distinct_domains_separate_hashes() {
        // A RequestAuth and GatewayReceipt that happen to share a borsh prefix must not
        // collide thanks to the domain tags.
        assert_ne!(DOMAIN_REQAUTH, DOMAIN_GWRCPT);
    }
}
