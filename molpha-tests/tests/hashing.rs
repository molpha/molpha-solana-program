//! Domain-separated hashes: the consumer request authorization and the three punishment evidence
//! hashes, which double as `PunishmentCase` PDA seeds.
//!
//! Pure unit tests — no VM. Each hash is asserted sensitive to every field it commits to, and
//! each domain is asserted distinct from the others, so no offence can be filed under another's
//! evidence hash.

use anchor_lang::prelude::Pubkey;
use molpha::state::{
    equivocation_evidence_hash, hash_request_auth, hash_request_auth_fields,
    commitment_reuse_evidence_hash, unsolicited_signing_evidence_hash, RequestAuth, SignedStatement,
    SigningContext, DOMAIN_EQUIVOCATION, DOMAIN_REQAUTH,
};

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
            &auth.source_id,
            auth.signatures_required,
            auth.timestamp
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
            &foreign.source_id,
            foreign.signatures_required,
            foreign.timestamp
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

fn fixture_context(source_byte: u8, signatures_required: u8, ts: u64) -> SigningContext {
    SigningContext {
        source_id: [source_byte; 32],
        signatures_required,
        canonical_timestamp: ts,
    }
}

#[test]
fn equivocation_hash_is_order_independent() {
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let forward = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);
    let backward = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &b, &a);
    assert_eq!(forward, backward);
}

#[test]
fn equivocation_hash_is_sensitive_to_every_field() {
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let base = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);

    assert_ne!(
        equivocation_evidence_hash(&[8u8; 32], 4, 3, 1_700_000_000, &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&[7u8; 32], 5, 3, 1_700_000_000, &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&[7u8; 32], 4, 9, 1_700_000_000, &a, &b),
        base
    );
    assert_ne!(
        equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_001, &a, &b),
        base
    );

    let mut a2 = a.clone();
    a2.value[0] ^= 0xff;
    assert_ne!(
        equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a2, &b),
        base
    );
}

#[test]
fn unsolicited_signing_hash_is_sensitive_to_every_field() {
    let a = fixture_statement(1, 0b0011);
    let base = unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a);

    assert_ne!(
        unsolicited_signing_evidence_hash(&[8u8; 32], 4, 3, 1_700_000_000, &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&[7u8; 32], 5, 3, 1_700_000_000, &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&[7u8; 32], 4, 9, 1_700_000_000, &a),
        base
    );
    assert_ne!(
        unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_001, &a),
        base
    );

    let mut a2 = a.clone();
    a2.value[0] ^= 0xff;
    assert_ne!(
        unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a2),
        base
    );
}

#[test]
fn offence_evidence_domains_do_not_collide() {
    // Same duty tuple and statements; domain prefix must still separate offence hashes.
    let a = fixture_statement(1, 0b0011);
    let b = fixture_statement(2, 0b0101);
    let ctx_a = fixture_context(7, 3, 1_700_000_000);
    let ctx_b = fixture_context(7, 3, 1_700_000_000);

    let equivocation = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);
    let unsolicited = unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a);
    let commitment_reuse = commitment_reuse_evidence_hash(4, &ctx_a, &a, &ctx_b, &b);

    assert_ne!(equivocation, unsolicited);
    assert_ne!(equivocation, commitment_reuse);
    assert_ne!(unsolicited, commitment_reuse);
}

#[test]
fn commitment_reuse_hash_is_pairwise_order_independent() {
    let ctx_a = fixture_context(7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let ctx_b = fixture_context(9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);

    let forward = commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);
    let backward = commitment_reuse_evidence_hash(4, &ctx_b, &stmt_b, &ctx_a, &stmt_a);
    assert_eq!(forward, backward);
}

#[test]
fn nonce_reuse_hash_pairs_each_context_with_its_own_statement() {
    let ctx_a = fixture_context(7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let ctx_b = fixture_context(9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);

    let paired = commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);
    let cross_paired = commitment_reuse_evidence_hash(4, &ctx_a, &stmt_b, &ctx_b, &stmt_a);
    assert_ne!(paired, cross_paired);
}

#[test]
fn nonce_reuse_hash_is_sensitive_to_every_field() {
    let ctx_a = fixture_context(7, 3, 1_700_000_000);
    let stmt_a = fixture_statement(1, 0b0011);
    let ctx_b = fixture_context(9, 4, 1_700_000_100);
    let stmt_b = fixture_statement(2, 0b0101);
    let base = commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);

    assert_ne!(
        commitment_reuse_evidence_hash(5, &ctx_a, &stmt_a, &ctx_b, &stmt_b),
        base
    );

    let mut ctx_a2 = ctx_a;
    ctx_a2.source_id = [8u8; 32];
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a2, &stmt_a, &ctx_b, &stmt_b),
        base
    );
    let mut ctx_a3 = ctx_a;
    ctx_a3.signatures_required = 9;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a3, &stmt_a, &ctx_b, &stmt_b),
        base
    );
    let mut ctx_a4 = ctx_a;
    ctx_a4.canonical_timestamp += 1;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a4, &stmt_a, &ctx_b, &stmt_b),
        base
    );

    let mut ctx_b2 = ctx_b;
    ctx_b2.source_id = [10u8; 32];
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b2, &stmt_b),
        base
    );
    let mut ctx_b3 = ctx_b;
    ctx_b3.signatures_required = 6;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b3, &stmt_b),
        base
    );
    let mut ctx_b4 = ctx_b;
    ctx_b4.canonical_timestamp += 1;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b4, &stmt_b),
        base
    );

    let mut stmt_a2 = stmt_a.clone();
    stmt_a2.value[0] ^= 0xff;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a2, &ctx_b, &stmt_b),
        base
    );
    let mut stmt_a3 = stmt_a.clone();
    stmt_a3.agg_sig_s[0] ^= 0xff;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a3, &ctx_b, &stmt_b),
        base
    );
    let mut stmt_a4 = stmt_a.clone();
    stmt_a4.commitment_addr[0] ^= 0xff;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a4, &ctx_b, &stmt_b),
        base
    );
    let mut stmt_a5 = stmt_a.clone();
    stmt_a5.signers_bitmap[0] ^= 0xff;
    assert_ne!(
        commitment_reuse_evidence_hash(4, &ctx_a, &stmt_a5, &ctx_b, &stmt_b),
        base
    );
}
