//! Domain-separated hashes: ticket identity, lottery and gateway dispatch messages, and the
//! punishment evidence hashes, which double as `PunishmentCase` PDA seeds.
//!
//! Pure unit tests — no VM. Each hash is asserted sensitive to every field it commits to, and
//! each domain is asserted distinct from the others, so no offence can be filed under another's
//! evidence hash.

use anchor_lang::prelude::Pubkey;
use molpha::state::{
    attestation_hash, commitment_batch_message_hash, commitment_leaf_hash, dispatch_message,
    ticket_hash, ticket_id, verify_commitment_merkle_proof, CommitmentReuseEvidence,
    CommittedNoncePair, EquivocationEvidence, Evidence, InactivityEvidence, NonceReuseEvidence,
    PartialTranscript, UnsolicitedSigningEvidence, COMMITMENT_MERKLE_NODE_PREFIX,
    DOMAIN_COMMITMENT_BATCH, DOMAIN_COMMITMENT_REUSE, DOMAIN_DISPATCH, DOMAIN_EQUIVOCATION,
    DOMAIN_INACTIVITY, DOMAIN_NONCE_REUSE, DOMAIN_TICKET, DOMAIN_TICKET_ID,
    DOMAIN_UNSOLICITED_SIGNING,
};
use molpha_tests::scenario::ZERO_COALITION_KEY;
use molpha_verifier::{Attestation, AttestationPayload, SchnorrSignature};
use solana_keccak_hasher::hashv;

fn fixture_ticket_attestation() -> Attestation {
    Attestation {
        payload: AttestationPayload {
            value: [0x42; 32],
            source_id: [0x11; 32],
            registry_version: 4,
            signatures_required: 3,
            // unix milliseconds
            timestamp: 1_700_000_000_000,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0x66; 32],
            commitment: [0x77; 20],
            signers_bitmap: [0x07; 32],
        },
    }
}

fn equivocation_evidence_hash(a: &Attestation, b: &Attestation) -> [u8; 32] {
    EquivocationEvidence {
        attestation_a: a.clone(),
        attestation_b: b.clone(),
        coalition_key_a: ZERO_COALITION_KEY,
        coalition_key_b: ZERO_COALITION_KEY,
    }
    .hash()
}

fn commitment_reuse_evidence_hash(a: &Attestation, b: &Attestation) -> [u8; 32] {
    CommitmentReuseEvidence {
        attestation_a: a.clone(),
        attestation_b: b.clone(),
        coalition_key_a: ZERO_COALITION_KEY,
        coalition_key_b: ZERO_COALITION_KEY,
    }
    .hash()
}

fn unsolicited_signing_evidence_hash(attestation: &Attestation) -> [u8; 32] {
    UnsolicitedSigningEvidence {
        attestation: attestation.clone(),
        coalition_key: ZERO_COALITION_KEY,
    }
    .hash()
}

#[allow(clippy::too_many_arguments)]
fn nonce_reuse_evidence_hash(
    registry_version: u32,
    node_index: u32,
    epoch: u64,
    batch_meta: &[u8; 32],
    batch_root: &[u8; 32],
    slot: u64,
    nonces: &CommittedNoncePair,
    transcript_a: &PartialTranscript,
    transcript_b: &PartialTranscript,
) -> [u8; 32] {
    NonceReuseEvidence {
        registry_version,
        node_index,
        epoch,
        batch_meta: *batch_meta,
        batch_root: *batch_root,
        batch_sig_s: [0u8; 32],
        batch_commitment_addr: [0u8; 20],
        slot,
        nonces: *nonces,
        merkle_proof: Vec::new(),
        transcript_a: *transcript_a,
        transcript_b: *transcript_b,
    }
    .hash()
}

/// Golden vectors shared with the CLI and the node client, which derive the same values
/// off-chain. A change here is a wire break.
#[test]
fn ticket_hashes_match_golden_vectors() {
    let attestation = fixture_ticket_attestation();
    assert_eq!(
        hex(&ticket_id(&attestation.payload)),
        TICKET_ID_GOLDEN,
        "ticket_id"
    );
    assert_eq!(
        hex(&ticket_hash(&attestation)),
        TICKET_HASH_GOLDEN,
        "ticket_hash"
    );
    assert_eq!(
        hex(&dispatch_message(&molpha::ID, &attestation.payload)),
        DISPATCH_MESSAGE_GOLDEN,
        "dispatch_message"
    );
}

const TICKET_ID_GOLDEN: &str = "1e5b3087286e8207743d03b04ec310a96b8c5250c45442fa6cfd77070c9940d8";
const TICKET_HASH_GOLDEN: &str = "a39fe3fd44902947ffc92501bf1019847a1ceda7c7e29157c8a1951676bce8d2";
const DISPATCH_MESSAGE_GOLDEN: &str =
    "7bcfad5c12968c95213e4b6ddf099e454125d81e3c6aa23510eadb11b33602c0";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn ticket_hashes_match_their_documented_preimages() {
    let attestation = fixture_ticket_attestation();
    let payload = &attestation.payload;
    assert_eq!(
        ticket_id(payload),
        hashv(&[
            DOMAIN_TICKET_ID,
            payload.source_id.as_ref(),
            &[payload.signatures_required],
            &payload.timestamp.to_be_bytes(),
        ])
        .to_bytes()
    );
    assert_eq!(
        ticket_hash(&attestation),
        hashv(&[
            DOMAIN_TICKET,
            attestation.message_hash().as_ref(),
            attestation.signature.agg_sig_s.as_ref(),
            attestation.signature.commitment.as_ref(),
        ])
        .to_bytes()
    );
    assert_eq!(
        dispatch_message(&molpha::ID, payload),
        hashv(&[
            DOMAIN_DISPATCH,
            molpha::ID.as_ref(),
            payload.source_id.as_ref(),
            &[payload.signatures_required],
            &payload.registry_version.to_be_bytes(),
            &payload.timestamp.to_be_bytes(),
        ])
        .to_bytes()
    );
}

/// `ticket_id` is round identity: alternative aggregates of one round (other signers, another
/// value, another nonce, another registry version) share it, so a round counts once.
#[test]
fn ticket_id_ignores_everything_but_the_round_identity() {
    let base = fixture_ticket_attestation();
    let id = ticket_id(&base.payload);

    let mut other_aggregate = base.clone();
    other_aggregate.payload.value = [0x43; 32];
    other_aggregate.payload.registry_version = 5;
    other_aggregate.signature.signers_bitmap = [0x0b; 32];
    other_aggregate.signature.agg_sig_s = [0x01; 32];
    assert_eq!(ticket_id(&other_aggregate.payload), id);

    let mut source = base.clone();
    source.payload.source_id[0] ^= 1;
    assert_ne!(ticket_id(&source.payload), id, "source_id");
    let mut quorum = base.clone();
    quorum.payload.signatures_required += 1;
    assert_ne!(ticket_id(&quorum.payload), id, "signatures_required");
    let mut time = base.clone();
    time.payload.timestamp += 1;
    assert_ne!(ticket_id(&time.payload), id, "timestamp");
}

/// The lottery covers the whole aggregate, so only the signers can know a round's outcome.
#[test]
fn ticket_hash_covers_message_and_signature() {
    let base = fixture_ticket_attestation();
    let hash = ticket_hash(&base);

    let mut value = base.clone();
    value.payload.value[0] ^= 1;
    assert_ne!(ticket_hash(&value), hash, "value");
    let mut bitmap = base.clone();
    bitmap.signature.signers_bitmap[0] ^= 1;
    assert_ne!(ticket_hash(&bitmap), hash, "signers_bitmap");
    let mut s = base.clone();
    s.signature.agg_sig_s[0] ^= 1;
    assert_ne!(ticket_hash(&s), hash, "agg_sig_s");
    let mut commitment = base.clone();
    commitment.signature.commitment[0] ^= 1;
    assert_ne!(ticket_hash(&commitment), hash, "commitment");
}

/// The dispatch signature binds the round a gateway admitted, on this deployment only. It is
/// fixed before signing, so it covers neither the value nor the signer set.
#[test]
fn dispatch_message_commits_to_the_admitted_round() {
    let base = fixture_ticket_attestation();
    let message = dispatch_message(&molpha::ID, &base.payload);

    let mut value = base.clone();
    value.payload.value[0] ^= 1;
    assert_eq!(dispatch_message(&molpha::ID, &value.payload), message);

    let mut version = base.clone();
    version.payload.registry_version += 1;
    assert_ne!(
        dispatch_message(&molpha::ID, &version.payload),
        message,
        "registry_version"
    );
    let mut time = base.clone();
    time.payload.timestamp += 1;
    assert_ne!(
        dispatch_message(&molpha::ID, &time.payload),
        message,
        "timestamp"
    );
    assert_ne!(
        dispatch_message(&Pubkey::new_from_array([0x99; 32]), &base.payload),
        message,
        "program_id"
    );
}

#[test]
fn distinct_domains_separate_hashes() {
    assert_ne!(DOMAIN_TICKET, DOMAIN_TICKET_ID);
    assert_ne!(DOMAIN_TICKET, DOMAIN_DISPATCH);
    assert_ne!(DOMAIN_TICKET_ID, DOMAIN_DISPATCH);
    assert_ne!(DOMAIN_DISPATCH, DOMAIN_EQUIVOCATION);
}

fn fixture_attestation(value: u8, bitmap_byte: u8) -> Attestation {
    Attestation {
        payload: AttestationPayload {
            value: [value; 32],
            source_id: [0x07; 32],
            registry_version: 4,
            signatures_required: 3,
            timestamp: 1_700_000_000,
        },
        signature: SchnorrSignature {
            agg_sig_s: [0x11; 32],
            commitment: [0x22; 20],
            signers_bitmap: {
                let mut bm = [0u8; 32];
                bm[31] = bitmap_byte;
                bm
            },
        },
    }
}

/// One mutation per attestation field, so every evidence hash can be checked against all of them.
type FieldMutation = (&'static str, fn(&mut Attestation));

const FIELD_MUTATIONS: [FieldMutation; 8] = [
    ("value", |a| a.payload.value[0] ^= 0xff),
    ("source_id", |a| a.payload.source_id[0] ^= 0xff),
    ("registry_version", |a| a.payload.registry_version += 1),
    ("signatures_required", |a| {
        a.payload.signatures_required += 1
    }),
    ("timestamp", |a| a.payload.timestamp += 1),
    ("agg_sig_s", |a| a.signature.agg_sig_s[0] ^= 0xff),
    ("commitment", |a| a.signature.commitment[0] ^= 0xff),
    ("signers_bitmap", |a| a.signature.signers_bitmap[0] ^= 0xff),
];

/// Pinned so off-chain reporters can derive the `PunishmentCase` seed without the program crate.
#[test]
fn attestation_hash_is_the_documented_field_concatenation() {
    let a = fixture_attestation(1, 0b0011);
    let expected = hashv(&[
        a.payload.value.as_slice(),
        a.payload.source_id.as_slice(),
        a.payload.registry_version.to_be_bytes().as_slice(),
        a.payload.signatures_required.to_be_bytes().as_slice(),
        a.payload.timestamp.to_be_bytes().as_slice(),
        a.signature.agg_sig_s.as_slice(),
        a.signature.commitment.as_slice(),
        a.signature.signers_bitmap.as_slice(),
    ])
    .to_bytes();
    assert_eq!(attestation_hash(&a), expected);

    let b = fixture_attestation(2, 0b0101);
    let (lo, hi) = {
        let (ha, hb) = (attestation_hash(&a), attestation_hash(&b));
        if ha <= hb {
            (ha, hb)
        } else {
            (hb, ha)
        }
    };
    assert_eq!(
        equivocation_evidence_hash(&a, &b),
        hashv(&[DOMAIN_EQUIVOCATION, &lo, &hi]).to_bytes()
    );
    assert_eq!(
        commitment_reuse_evidence_hash(&a, &b),
        hashv(&[DOMAIN_COMMITMENT_REUSE, &lo, &hi]).to_bytes()
    );
    assert_eq!(
        unsolicited_signing_evidence_hash(&a),
        hashv(&[DOMAIN_UNSOLICITED_SIGNING, &attestation_hash(&a)]).to_bytes()
    );
}

#[test]
fn equivocation_hash_is_order_independent() {
    let a = fixture_attestation(1, 0b0011);
    let b = fixture_attestation(2, 0b0101);
    assert_eq!(
        equivocation_evidence_hash(&a, &b),
        equivocation_evidence_hash(&b, &a)
    );
}

#[test]
fn equivocation_hash_is_sensitive_to_every_field() {
    let a = fixture_attestation(1, 0b0011);
    let b = fixture_attestation(2, 0b0101);
    let base = equivocation_evidence_hash(&a, &b);

    for (field, mutate) in FIELD_MUTATIONS {
        let (mut a2, mut b2) = (a.clone(), b.clone());
        mutate(&mut a2);
        mutate(&mut b2);
        assert_ne!(equivocation_evidence_hash(&a2, &b), base, "{field} of a");
        assert_ne!(equivocation_evidence_hash(&a, &b2), base, "{field} of b");
    }
}

/// Coalition keys are verification inputs, not evidence: the case PDA must derive from the
/// attestations alone, so a watcher can find a case without knowing the keys the reporter sent.
#[test]
fn evidence_hashes_ignore_coalition_keys() {
    let a = fixture_attestation(1, 0b0011);
    let b = fixture_attestation(2, 0b0101);
    let key = molpha_verifier::CoalitionKey {
        x: [0x11; 32],
        y: [0x22; 32],
    };

    let equivocation = EquivocationEvidence {
        attestation_a: a.clone(),
        attestation_b: b.clone(),
        coalition_key_a: key,
        coalition_key_b: key,
    };
    assert_eq!(equivocation.hash(), equivocation_evidence_hash(&a, &b));

    let commitment_reuse = CommitmentReuseEvidence {
        attestation_a: a.clone(),
        attestation_b: b.clone(),
        coalition_key_a: key,
        coalition_key_b: key,
    };
    assert_eq!(
        commitment_reuse.hash(),
        commitment_reuse_evidence_hash(&a, &b)
    );

    let unsolicited = UnsolicitedSigningEvidence {
        attestation: a.clone(),
        coalition_key: key,
    };
    assert_eq!(unsolicited.hash(), unsolicited_signing_evidence_hash(&a));
}

#[test]
fn unsolicited_signing_hash_is_sensitive_to_every_field() {
    let a = fixture_attestation(1, 0b0011);
    let base = unsolicited_signing_evidence_hash(&a);

    for (field, mutate) in FIELD_MUTATIONS {
        let mut a2 = a.clone();
        mutate(&mut a2);
        assert_ne!(unsolicited_signing_evidence_hash(&a2), base, "{field}");
    }
}

#[test]
fn offence_evidence_domains_do_not_collide() {
    // Same attestations; the domain prefix must still separate offence hashes.
    let a = fixture_attestation(1, 0b0011);
    let b = fixture_attestation(2, 0b0101);

    let equivocation = equivocation_evidence_hash(&a, &b);
    let unsolicited = unsolicited_signing_evidence_hash(&a);
    let commitment_reuse = commitment_reuse_evidence_hash(&a, &b);

    assert_ne!(equivocation, unsolicited);
    assert_ne!(equivocation, commitment_reuse);
    assert_ne!(unsolicited, commitment_reuse);

    let nonce_reuse = fixture_nonce_reuse_hash();
    assert_ne!(equivocation, nonce_reuse);
    assert_ne!(unsolicited, nonce_reuse);
    assert_ne!(commitment_reuse, nonce_reuse);
    assert_ne!(DOMAIN_NONCE_REUSE, DOMAIN_COMMITMENT_REUSE);
    assert_ne!(DOMAIN_NONCE_REUSE, DOMAIN_COMMITMENT_BATCH);
    assert_ne!(DOMAIN_INACTIVITY, DOMAIN_EQUIVOCATION);
    assert_ne!(DOMAIN_INACTIVITY, DOMAIN_NONCE_REUSE);

    let inactivity = inactivity_evidence_hash(4, 1);
    assert_ne!(DOMAIN_INACTIVITY, DOMAIN_TICKET);
    assert_ne!(equivocation, inactivity);
    assert_ne!(unsolicited, inactivity);
    assert_ne!(commitment_reuse, inactivity);
    assert_ne!(nonce_reuse, inactivity);
}

fn inactivity_evidence_hash(epoch: u64, roster_slot: u16) -> [u8; 32] {
    InactivityEvidence {
        epoch,
        roster_slot,
        registry_version: 9,
        node_index: 2,
    }
    .hash()
}

/// One case per node per epoch: the hash names the epoch and roster slot only, so the same
/// outage cannot be refiled after reinstatement moves the node to a new version and index.
#[test]
fn inactivity_hash_commits_to_the_epoch_and_roster_slot_only() {
    let base = inactivity_evidence_hash(4, 1);
    assert_ne!(inactivity_evidence_hash(5, 1), base, "epoch");
    assert_ne!(inactivity_evidence_hash(4, 2), base, "roster_slot");

    let relocated = InactivityEvidence {
        epoch: 4,
        roster_slot: 1,
        registry_version: 12,
        node_index: 7,
    };
    assert_eq!(
        relocated.hash(),
        base,
        "registry location is not part of the case"
    );
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
    let mut a = fixture_attestation(1, 0b0011);
    a.payload.source_id = [0x09; 32];
    let b = fixture_attestation(2, 0b0101);

    assert_eq!(
        commitment_reuse_evidence_hash(&a, &b),
        commitment_reuse_evidence_hash(&b, &a)
    );
}

#[test]
fn commitment_reuse_hash_is_sensitive_to_every_field() {
    let mut a = fixture_attestation(1, 0b0011);
    a.payload.source_id = [0x09; 32];
    let b = fixture_attestation(2, 0b0101);
    let base = commitment_reuse_evidence_hash(&a, &b);

    for (field, mutate) in FIELD_MUTATIONS {
        let (mut a2, mut b2) = (a.clone(), b.clone());
        mutate(&mut a2);
        mutate(&mut b2);
        assert_ne!(
            commitment_reuse_evidence_hash(&a2, &b),
            base,
            "{field} of a"
        );
        assert_ne!(
            commitment_reuse_evidence_hash(&a, &b2),
            base,
            "{field} of b"
        );
    }
}
