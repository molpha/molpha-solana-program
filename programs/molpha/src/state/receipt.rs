use anchor_lang::prelude::*;
use solana_sha256_hasher::hashv as sha256_hashv;

pub const DOMAIN_REQAUTH: &[u8] = b"MOLPHA_REQAUTH_V1";
pub const DOMAIN_GWRCPT: &[u8] = b"MOLPHA_GWRCPT_V1";
pub const DOMAIN_AGENT_REQAUTH: &[u8] = b"MOLPHA_AGENT_REQAUTH_V1";

/// Consumer demand authorization (Ed25519-signed by `consumer.authority`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct RequestAuth {
    pub feed_id: [u8; 32],
    pub timestamp: u64,
}

/// The committed aggregate Schnorr signature. **Not verified on the happy path.**
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

/// `H(RequestAuth)` — domain-separated SHA-256 over the borsh encoding.
pub fn hash_request_auth(request_auth: &RequestAuth) -> [u8; 32] {
    let body = request_auth
        .try_to_vec()
        .expect("RequestAuth serialization is infallible");
    sha256_hashv(&[DOMAIN_REQAUTH, &body]).to_bytes()
}

/// `H(RequestAuth)` from the instruction-level `(feed_id, timestamp)` pair.
pub fn hash_request_auth_fields(feed_id: &[u8; 32], timestamp: u64) -> [u8; 32] {
    hash_request_auth(&RequestAuth {
        feed_id: *feed_id,
        timestamp,
    })
}

/// Agent demand authorization (Ed25519-signed by the escrow `authority` / x402 payer).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentRequestAuth {
    /// The `AgentEscrow` PDA address.
    pub agent: Pubkey,
    pub request_nonce: u64,
    pub feed_id: [u8; 32],
    /// Max price (USDC atomic units) the payer authorizes for this request.
    pub max_price: u64,
}

/// `H(AgentRequestAuth)` — domain-separated SHA-256 over the borsh encoding.
pub fn hash_agent_request_auth(auth: &AgentRequestAuth) -> [u8; 32] {
    let body = auth
        .try_to_vec()
        .expect("AgentRequestAuth serialization is infallible");
    sha256_hashv(&[DOMAIN_AGENT_REQAUTH, &body]).to_bytes()
}

/// `H(AgentRequestAuth)` from instruction-level fields.
pub fn hash_agent_request_auth_fields(
    agent: &Pubkey,
    request_nonce: u64,
    feed_id: &[u8; 32],
    max_price: u64,
) -> [u8; 32] {
    hash_agent_request_auth(&AgentRequestAuth {
        agent: *agent,
        request_nonce,
        feed_id: *feed_id,
        max_price,
    })
}

/// `sha256(AggregatedSig)` — the commitment stored in `RoundReceiptRecord`.
pub fn hash_agg_sig(agg_sig: &AggregatedSig) -> [u8; 32] {
    let body = agg_sig
        .try_to_vec()
        .expect("AggregatedSig serialization is infallible");
    sha256_hashv(&[&body]).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_request_auth() -> RequestAuth {
        RequestAuth {
            feed_id: [0x11; 32],
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

    #[test]
    fn hash_request_auth_fields_matches_struct_hash() {
        let auth = fixture_request_auth();
        assert_eq!(
            hash_request_auth_fields(&auth.feed_id, auth.timestamp),
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
        b.feed_id[0] ^= 0x01;
        assert_ne!(hash_request_auth(&b), base);
    }

    #[test]
    fn agg_sig_hash_changes_with_bitmap() {
        let base = hash_agg_sig(&fixture_agg_sig());
        let mut a = fixture_agg_sig();
        a.signers_bitmap[0] ^= 0x01;
        assert_ne!(hash_agg_sig(&a), base);
    }

    fn fixture_agent_request_auth() -> AgentRequestAuth {
        AgentRequestAuth {
            agent: Pubkey::new_from_array([0x22; 32]),
            request_nonce: 42,
            feed_id: [0x11; 32],
            max_price: 1_030,
        }
    }

    #[test]
    fn hash_agent_request_auth_fields_matches_struct_hash() {
        let auth = fixture_agent_request_auth();
        assert_eq!(
            hash_agent_request_auth_fields(
                &auth.agent,
                auth.request_nonce,
                &auth.feed_id,
                auth.max_price,
            ),
            hash_agent_request_auth(&auth)
        );
    }

    #[test]
    fn agent_request_auth_hash_is_field_sensitive() {
        let base = hash_agent_request_auth(&fixture_agent_request_auth());
        let mut a = fixture_agent_request_auth();
        a.request_nonce += 1;
        assert_ne!(hash_agent_request_auth(&a), base);
        let mut b = fixture_agent_request_auth();
        b.max_price += 1;
        assert_ne!(hash_agent_request_auth(&b), base);
    }

    #[test]
    fn distinct_domains_separate_hashes() {
        // Domain tags must not collide — they bind hashes to their role.
        assert_ne!(DOMAIN_REQAUTH, DOMAIN_GWRCPT);
        assert_ne!(DOMAIN_REQAUTH, DOMAIN_AGENT_REQAUTH);
    }
}
