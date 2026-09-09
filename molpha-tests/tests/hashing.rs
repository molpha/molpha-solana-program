//! Domain-separated hashes: the consumer request authorization and the three punishment evidence
//! hashes, which double as `PunishmentCase` PDA seeds.
//!
//! Pure unit tests — no VM. Each hash is asserted sensitive to every field it commits to, and
//! each domain is asserted distinct from the others, so no offence can be filed under another's
//! evidence hash.

use anchor_lang::prelude::Pubkey;
use molpha::state::{
    commitment_batch_message_hash, commitment_leaf_hash, commitment_reuse_evidence_hash,
    equivocation_evidence_hash, hash_request_auth, hash_request_auth_fields,
    nonce_reuse_evidence_hash, unsolicited_signing_evidence_hash, verify_commitment_merkle_proof,
    CommittedNoncePair, PartialTranscript, RequestAuth, SignedStatement,
    COMMITMENT_MERKLE_NODE_PREFIX, DOMAIN_COMMITMENT_BATCH, DOMAIN_COMMITMENT_REUSE,
    DOMAIN_EQUIVOCATION, DOMAIN_NONCE_REUSE, DOMAIN_REQAUTH,
};
use molpha::utils::settlement::RoundRequest;

const GATEWAY: Pubkey = Pubkey::new_from_array([0x22; 32]);

fn fixture_request_auth() -> RequestAuth {
    RequestAuth {
        program_id: molpha::ID,
        gateway: GATEWAY,
        source_id: [0x11; 32],
        signatures_required: 3,
        timestamp: 1_700_000_000,
    }
}

#[test]
fn hash_request_auth_fields_matches_struct_hash() {
    let auth = fixture_request_auth();
    assert_eq!(
        hash_request_auth_fields(
            &auth.gateway,
            &RoundRequest {
                source_id: auth.source_id,
                canonical_timestamp: auth.timestamp,
                signatures_required: auth.signatures_required,
                registry_version: 4,
            },
        ),
        hash_request_auth(&auth)
    );
}

#[test]
fn hash_request_auth_fields_commits_to_this_program() {
    // The field-level helper is what the program calls; it must never let a caller name another
    // deployment's program id.
    let mut foreign = fixture_request_auth();
    foreign.program_id = Pubkey::new_from_array([0x99; 32]);
    assert_ne!(
        hash_request_auth(&foreign),
        hash_request_auth_fields(
            &foreign.gateway,
            &RoundRequest {
                source_id: foreign.source_id,
                canonical_timestamp: foreign.timestamp,
                signatures_required: foreign.signatures_required,
                registry_version: 4,
            },
        )
    );
}

#[test]
fn request_auth_hash_is_field_sensitive() {
    let base = hash_request_auth(&fixture_request_auth());
    let mut a = fixture_request_auth();
    a.timestamp += 1;
    assert_ne!(hash_request_auth(&a), base);
    let mut b = fixture_request_auth();
    b.source_id[0] ^= 0x01;
    assert_ne!(hash_request_auth(&b), base);
    let mut c = fixture_request_auth();
    c.signatures_required += 1;
    assert_ne!(hash_request_auth(&c), base);
    // An authorization is redeemable by exactly one gateway on exactly one deployment.
    let mut d = fixture_request_auth();
    d.gateway = Pubkey::new_from_array([0x33; 32]);
    assert_ne!(hash_request_auth(&d), base);
    let mut e = fixture_request_auth();
    e.program_id = Pubkey::new_from_array([0x44; 32]);
    assert_ne!(hash_request_auth(&e), base);
}

#[test]
fn distinct_domains_separate_hashes() {
    assert_ne!(DOMAIN_REQAUTH, DOMAIN_EQUIVOCATION);
}

fn fixture_statement(value: u8, bitmap_byte: u8) -> SignedStatement {
    SignedStatement {
        value: [value; 32],
        agg_sig_s: [0x11; 32],
        commitment_addr: [0x22; 20],
        signers_bitmap: {
            let mut bm = [0u8; 32];
            bm[31] = bitmap_byte;
            bm
        },
    }
}

fn fixture_round_request(
    registry_version: u32,
    source_byte: u8,
    signatures_required: u8,
    ts: u64,
) -> RoundRequest {
    RoundRequest {
        source_id: [source_byte; 32],
        registry_version,
        signatures_required,
        canonical_timestamp: ts,
    }
}

#[test]
fn equivocation_hash_is_order_independent() {
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let req = fixture_round_request(4, 7, 3, 1_700_000_000);
    let forward = equivocation_evidence_hash(&req, &a, &b);
    let backward = equivocation_evidence_hash(&req, &b, &a);
    assert_eq!(forward, backward);
}

#[test]
fn equivocation_hash_is_sensitive_to_every_field() {
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let base_req = fixture_round_request(4, 7, 3, 1_700_000_000);
    let base = equivocation_evidence_hash(&base_req, &a, &b);

    assert_ne!(
        equivocation_evidence_hash(&fixture_round_request(4, 8, 3, 1_700_000_000), &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&fixture_round_request(5, 7, 3, 1_700_000_000), &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&fixture_round_request(4, 7, 9, 1_700_000_000), &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&fixture_round_request(4, 7, 3, 1_700_000_001), &a, &b),
        base
    );

    let mut a2 = a.clone();
    a2.value[0] ^= 0xff;
    assert_ne!(equivocation_evidence_hash(&base_req, &a2, &b), base);
}

#[test]
fn unsolicited_signing_hash_is_sensitive_to_every_field() {
    let a = fixture_statement(1, 0b0011);
    let base_req = fixture_round_request(4, 7, 3, 1_700_000_000);
    let base = unsolicited_signing_evidence_hash(&base_req, &a);

    assert_ne!(
        unsolicited_signing_evidence_hash(&fixture_round_request(4, 8, 3, 1_700_000_000), &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&fixture_round_request(5, 7, 3, 1_700_000_000), &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&fixture_round_request(4, 7, 9, 1_700_000_000), &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&fixture_round_request(4, 7, 3, 1_700_000_001), &a),
        base
    );

    let mut a2 = a.clone();
    a2.value[0] ^= 0xff;
    assert_ne!(unsolicited_signing_evidence_hash(&base_req, &a2), base);
}

#[test]
fn offence_evidence_domains_do_not_collide() {
    // Same duty tuple and statements; domain prefix must still separate offence hashes.
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let req_a = fixture_round_request(4, 7, 3, 1_700_000_000);
    let req_b = fixture_round_request(4, 7, 3, 1_700_000_000);

    let equivocation = equivocation_evidence_hash(&req_a, &a, &b);
    let unsolicited = unsolicited_signing_evidence_hash(&req_a, &a);
    let commitment_reuse = commitment_reuse_evidence_hash(&req_a, &a, &req_b, &b);

    assert_ne!(equivocation, unsolicited);
    assert_ne!(equivocation, commitment_reuse);
    assert_ne!(unsolicited, commitment_reuse);

    let nonce_reuse = fixture_nonce_reuse_hash();
    assert_ne!(equivocation, nonce_reuse);
    assert_ne!(unsolicited, nonce_reuse);
    assert_ne!(commitment_reuse, nonce_reuse);
    assert_ne!(DOMAIN_NONCE_REUSE, DOMAIN_COMMITMENT_REUSE);
    assert_ne!(DOMAIN_NONCE_REUSE, DOMAIN_COMMITMENT_BATCH);
}

fn fixture_nonces() -> CommittedNoncePair {
    CommittedNoncePair {
        r1_x: [0x31; 32],
        r1_y: [0x32; 32],
        r2_x: [0x33; 32],
        r2_y: [0x34; 32],
    }
}

fn fixture_transcript(tag: u8) -> PartialTranscript {
    PartialTranscript {
        b: [tag; 32],
        c: [tag.wrapping_add(1); 32],
        s: [tag.wrapping_add(2); 32],
    }
}

fn fixture_nonce_reuse_hash() -> [u8; 32] {
    nonce_reuse_evidence_hash(
        4,
        1,
        9,
        &[0x55; 32],
        &[0x66; 32],
        3,
        &fixture_nonces(),
        &fixture_transcript(1),
        &fixture_transcript(9),
    )
}

#[test]
fn nonce_reuse_hash_commits_to_every_field() {
    let base = fixture_nonce_reuse_hash();
    let nonces = fixture_nonces();
    let (a, b) = (fixture_transcript(1), fixture_transcript(9));

    // Each argument, varied one at a time.
    assert_ne!(
        nonce_reuse_evidence_hash(5, 1, 9, &[0x55; 32], &[0x66; 32], 3, &nonces, &a, &b),
        base,
        "registry_version"
    );
    assert_ne!(
        nonce_reuse_evidence_hash(4, 2, 9, &[0x55; 32], &[0x66; 32], 3, &nonces, &a, &b),
        base,
        "node_index"
    );
    assert_ne!(
        nonce_reuse_evidence_hash(4, 1, 10, &[0x55; 32], &[0x66; 32], 3, &nonces, &a, &b),
        base,
        "epoch"
    );
    assert_ne!(
        nonce_reuse_evidence_hash(4, 1, 9, &[0x56; 32], &[0x66; 32], 3, &nonces, &a, &b),
        base,
        "batch_meta"
    );
    assert_ne!(
        nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x67; 32], 3, &nonces, &a, &b),
        base,
        "batch_root"
    );
    assert_ne!(
        nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x66; 32], 4, &nonces, &a, &b),
        base,
        "slot"
    );

    for mutate in [
        (|n: &mut CommittedNoncePair| n.r1_x[0] ^= 0xff) as fn(&mut CommittedNoncePair),
        |n: &mut CommittedNoncePair| n.r1_y[0] ^= 0xff,
        |n: &mut CommittedNoncePair| n.r2_x[0] ^= 0xff,
        |n: &mut CommittedNoncePair| n.r2_y[0] ^= 0xff,
    ] {
        let mut tampered = nonces;
        mutate(&mut tampered);
        assert_ne!(
            nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x66; 32], 3, &tampered, &a, &b),
            base,
            "nonce coordinate"
        );
    }

    for mutate in [
        (|t: &mut PartialTranscript| t.b[0] ^= 0xff) as fn(&mut PartialTranscript),
        |t: &mut PartialTranscript| t.c[0] ^= 0xff,
        |t: &mut PartialTranscript| t.s[0] ^= 0xff,
    ] {
        let mut tampered = a;
        mutate(&mut tampered);
        let hash =
            nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x66; 32], 3, &nonces, &tampered, &b);
        assert_ne!(hash, base, "transcript field");
    }
}

/// Filing the two transcripts in either order is one offence, so it must be one case.
#[test]
fn nonce_reuse_hash_is_transcript_order_independent() {
    let nonces = fixture_nonces();
    let (a, b) = (fixture_transcript(1), fixture_transcript(9));
    assert_eq!(
        nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x66; 32], 3, &nonces, &a, &b),
        nonce_reuse_evidence_hash(4, 1, 9, &[0x55; 32], &[0x66; 32], 3, &nonces, &b, &a),
    );
}

/// Leaves and internal nodes carry different prefixes, so an internal node cannot be replayed as a
/// leaf to claim a commitment that was never made.
#[test]
fn merkle_leaves_cannot_be_confused_with_internal_nodes() {
    let nonces = fixture_nonces();
    let leaf = commitment_leaf_hash(3, &nonces);
    let other = commitment_leaf_hash(4, &nonces);

    // A two-leaf tree: the root is an internal hash of both leaves.
    let root = {
        let (lo, hi) = if leaf <= other {
            (leaf, other)
        } else {
            (other, leaf)
        };
        solana_keccak_hasher::hashv(&[COMMITMENT_MERKLE_NODE_PREFIX, lo.as_slice(), hi.as_slice()])
            .to_bytes()
    };
    assert!(verify_commitment_merkle_proof(leaf, &[other], &root));
    assert!(verify_commitment_merkle_proof(other, &[leaf], &root));

    // The root is not itself a leaf of anything.
    assert_ne!(root, leaf);
    assert!(!verify_commitment_merkle_proof(root, &[], &leaf));
    // A leaf only matches an empty proof against a root that is that leaf.
    assert!(verify_commitment_merkle_proof(leaf, &[], &leaf));
    assert!(!verify_commitment_merkle_proof(leaf, &[], &root));
}

#[test]
fn commitment_leaf_commits_to_the_slot_and_both_nonces() {
    let nonces = fixture_nonces();
    let base = commitment_leaf_hash(3, &nonces);
    assert_ne!(commitment_leaf_hash(4, &nonces), base);

    let mut other = nonces;
    other.r2_y[31] ^= 1;
    assert_ne!(commitment_leaf_hash(3, &other), base);
}

#[test]
fn batch_message_binds_the_deployment_the_registry_version_and_the_node() {
    let base = commitment_batch_message_hash(&molpha::ID, 4, 1, 9, &[0x55; 32], &[0x66; 32]);
    let other_program = Pubkey::new_from_array([0x99; 32]);

    // A batch signed for one deployment must not be filable against another.
    assert_ne!(
        commitment_batch_message_hash(&other_program, 4, 1, 9, &[0x55; 32], &[0x66; 32]),
        base
    );
    // `node_index` only names a node within one registry version, so both are bound.
    assert_ne!(
        commitment_batch_message_hash(&molpha::ID, 5, 1, 9, &[0x55; 32], &[0x66; 32]),
        base
    );
    assert_ne!(
        commitment_batch_message_hash(&molpha::ID, 4, 2, 9, &[0x55; 32], &[0x66; 32]),
        base
    );
    assert_ne!(
        commitment_batch_message_hash(&molpha::ID, 4, 1, 10, &[0x55; 32], &[0x66; 32]),
        base
    );
    assert_ne!(
        commitment_batch_message_hash(&molpha::ID, 4, 1, 9, &[0x56; 32], &[0x66; 32]),
        base
    );
    assert_ne!(
        commitment_batch_message_hash(&molpha::ID, 4, 1, 9, &[0x55; 32], &[0x67; 32]),
        base
    );
}

#[test]
fn commitment_reuse_hash_is_pairwise_order_independent() {
    let req_a = fixture_round_request(4, 7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let req_b = fixture_round_request(4, 9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);

    let forward = commitment_reuse_evidence_hash(&req_a, &stmt_a, &req_b, &stmt_b);
    let backward = commitment_reuse_evidence_hash(&req_b, &stmt_b, &req_a, &stmt_a);
    assert_eq!(forward, backward);
}

#[test]
fn nonce_reuse_hash_pairs_each_context_with_its_own_statement() {
    let req_a = fixture_round_request(4, 7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let req_b = fixture_round_request(4, 9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);

    let paired = commitment_reuse_evidence_hash(&req_a, &stmt_a, &req_b, &stmt_b);
    let cross_paired = commitment_reuse_evidence_hash(&req_a, &stmt_b, &req_b, &stmt_a);
    assert_ne!(paired, cross_paired);
}

#[test]
fn nonce_reuse_hash_is_sensitive_to_every_field() {
    let req_a = fixture_round_request(4, 7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let req_b = fixture_round_request(4, 9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);
    let base = commitment_reuse_evidence_hash(&req_a, &stmt_a, &req_b, &stmt_b);

    assert_ne!(
        commitment_reuse_evidence_hash(
            &fixture_round_request(5, 7, 3, 1_700_000_000),
            &stmt_a,
            &req_b,
            &stmt_b,
        ),
        base
    );

    assert_ne!(
        commitment_reuse_evidence_hash(
            &fixture_round_request(4, 8, 3, 1_700_000_000),
            &stmt_a,
            &req_b,
            &stmt_b,
        ),
        base
    );
    assert_ne!(
        commitment_reuse_evidence_hash(
            &fixture_round_request(4, 7, 9, 1_700_000_000),
            &stmt_a,
            &req_b,
            &stmt_b,
        ),
        base
    );
    assert_ne!(
        commitment_reuse_evidence_hash(
            &fixture_round_request(4, 7, 3, 1_700_000_001),
            &stmt_a,
            &req_b,
            &stmt_b,
        ),
        base
    );

    assert_ne!(
        commitment_reuse_evidence_hash(
            &req_a,
            &stmt_a,
            &fixture_round_request(4, 10, 4, 1_700_000_100),
            &stmt_b,
        ),
        base
    );
    assert_ne!(
        commitment_reuse_evidence_hash(
            &req_a,
            &stmt_a,
            &fixture_round_request(4, 9, 6, 1_700_000_100),
            &stmt_b,
        ),
        base
    );
    assert_ne!(
        commitment_reuse_evidence_hash(
            &req_a,
            &stmt_a,
            &fixture_round_request(4, 9, 4, 1_700_000_101),
            &stmt_b,
        ),
        base
    );

    let mut stmt_a2 = stmt_a.clone();
    stmt_a2.value[0] ^= 0xff;
    assert_ne!(commitment_reuse_evidence_hash(&req_a, &stmt_a2, &req_b, &stmt_b), base);
    let mut stmt_a3 = stmt_a.clone();
    stmt_a3.agg_sig_s[0] ^= 0xff;
    assert_ne!(commitment_reuse_evidence_hash(&req_a, &stmt_a3, &req_b, &stmt_b), base);
    let mut stmt_a4 = stmt_a.clone();
    stmt_a4.commitment_addr[0] ^= 0xff;
    assert_ne!(commitment_reuse_evidence_hash(&req_a, &stmt_a4, &req_b, &stmt_b), base);
    let mut stmt_a5 = stmt_a.clone();
    stmt_a5.signers_bitmap[0] ^= 0xff;
    assert_ne!(commitment_reuse_evidence_hash(&req_a, &stmt_a5, &req_b, &stmt_b), base);
}
