//! `rotate_node_key`: replacing a node's signing key.
//!
//! A rotation publishes a registry version that names the new key at the node's existing index.
//! Because every version keeps the key it was created with, nothing signed before a rotation
//! stops verifying after it: the history tests below pin that for attestations, tickets and each
//! kind of slashing evidence.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use anchor_lang::prelude::Pubkey;
use libsecp256k1::PublicKey;
use molpha::error::MolphaError;
use molpha::events::NodeKeyRotated;
use molpha::state::{EquivocationEvidence, Node, NodeStatus, Secp256k1KeyClaim};
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 6;
const SIGS: u8 = 4;
/// `max_settlement_delay_seconds` of the default protocol: the rotation cooldown and the grace a
/// replaced registry version stays valid for.
const WINDOW_SECONDS: i64 = 604_800;

fn protocol(domain: &str) -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes(domain, NODE_COUNT);
    p
}

fn fresh_key(domain: &str) -> NodeKey {
    crypto::node_keys(domain, 1).remove(0)
}

/// The key `version`'s snapshot names for `node`.
fn snapshot_key(p: &Protocol, version: u32, node: &Pubkey) -> ([u8; 32], [u8; 32]) {
    let registry = p.env.registry(version);
    registry.keys[usize::from(registry.index_of(node).expect("node is a member"))]
}

fn current_key_of(p: &Protocol, node: &Pubkey) -> ([u8; 32], [u8; 32]) {
    let node: Node = p.env.get(node);
    (node.secp256k1_pubkey_x, node.secp256k1_pubkey_y)
}

fn claim_owner(p: &Protocol, key: &NodeKey) -> Pubkey {
    let claim: Secp256k1KeyClaim = p.env.get(&pda::secp256k1_key_claim(&key.compressed));
    claim.node_pubkey
}

fn index_of(p: &Protocol, node: &Pubkey) -> usize {
    p.nodes
        .iter()
        .position(|n| n.pda == *node)
        .expect("registered node")
}

/// A duty that selects `node` at the live registry version.
fn duty_selecting(p: &Protocol, node: &Pubkey) -> Duty {
    p.duties_selecting(node, 1, 0x5e).remove(0)
}

/// `signed`, re-signed with `substitute` standing in for `node`'s snapshot key, the way a signer
/// holding a stale or premature key would produce it. The coalition key is left as the snapshot's.
fn signed_with_substituted_key(
    p: &Protocol,
    signed: &SignedAttestation,
    node: &Pubkey,
    substitute: &NodeKey,
) -> SignedAttestation {
    let registry = p.env.registry(signed.duty.registry_version);
    let keys: Vec<&NodeKey> = crypto::bitmap_bits(&signed.signers_bitmap)
        .into_iter()
        .map(|bit| {
            let member = registry.nodes[bit];
            if member == *node {
                substitute
            } else {
                let registered = p.nodes.iter().find(|n| n.pda == member).unwrap();
                registered.key_for(&registry.keys[bit]).unwrap()
            }
        })
        .collect();
    let mut forged = signed.clone();
    forged.signature = crypto::sign_aggregate(&keys, &signed.message_hash, None);
    forged
}

// ---------------------------------------------------------------------------
// the rotation itself
// ---------------------------------------------------------------------------

#[test]
fn rotating_publishes_a_version_with_the_new_key_at_the_same_index() {
    let mut p = protocol("rotate-happy");
    let index = 2;
    let node_pda = p.nodes[index].pda;
    let owner = p.nodes[index].owner.pubkey();
    let old_key = p.nodes[index].key.clone();
    let new_key = fresh_key("rotate-happy-new");
    let before = p.current_version();
    let before_snapshot = p.env.registry(before);

    let meta = p.rotate_node_key(index, new_key.clone());
    let now = p.env.unix_timestamp();

    // Exactly one new version, with the same members in the same order and policy.
    assert_eq!(p.current_version(), before + 1);
    let after = p.env.registry(before + 1);
    assert_eq!(after.version, before + 1);
    assert_eq!(after.nodes, before_snapshot.nodes);
    assert_eq!(after.redundancy_buffer, before_snapshot.redundancy_buffer);
    assert_eq!(after.active_from, now);
    assert_eq!(after.grace_active_until, 0);
    for bit in 0..NODE_COUNT {
        let expected = if bit == index {
            (new_key.x, new_key.y)
        } else {
            before_snapshot.keys[bit]
        };
        assert_eq!(after.keys[bit], expected, "member {bit}");
    }

    // The version it replaces still names the old key, and goes into grace.
    let replaced = p.env.registry(before);
    assert_eq!(replaced.keys[index], (old_key.x, old_key.y));
    assert_eq!(replaced.grace_active_until, now + WINDOW_SECONDS);

    // The node account carries the new key, and nothing else about it moved.
    let node: Node = p.env.get(&node_pda);
    assert_eq!(
        (node.secp256k1_pubkey_x, node.secp256k1_pubkey_y),
        (new_key.x, new_key.y)
    );
    assert_eq!(node.key_rotated_at, now);
    assert_eq!(node.authority, owner);
    assert_eq!(node.status, NodeStatus::Active);
    assert_eq!(node.locked_amount, p.args.minimum_node_deposit);

    // The new key is reserved for the node, and the retired one stays reserved.
    assert_eq!(claim_owner(&p, &new_key), node_pda);
    assert_eq!(claim_owner(&p, &old_key), node_pda);

    let event: NodeKeyRotated = only_event(&meta);
    assert_eq!(event.node, node_pda);
    assert_eq!(event.node_authority, owner);
    assert_eq!(event.index, index as u32);
    assert_eq!(event.registry_version, before + 1);
    assert_eq!(
        (event.old_pubkey_x, event.old_pubkey_y),
        (old_key.x, old_key.y)
    );
    assert_eq!(
        (event.new_pubkey_x, event.new_pubkey_y),
        (new_key.x, new_key.y)
    );
    assert_eq!(event.rotated_at, now);
}

#[test]
fn only_the_node_authority_can_rotate() {
    let mut p = protocol("rotate-authority");
    let new_key = fresh_key("rotate-authority-new");
    let proof = p.key_rotation_proof(0, &new_key);

    // Neither a stranger nor the protocol authority has a node of their own to rotate.
    let stranger = p.env.keypair();
    let protocol_authority = p.authority.insecure_clone();
    for signer in [&stranger, &protocol_authority] {
        let ix = p.rotate_node_key_raw_ix(&signer.pubkey(), proof.clone(), p.current_version() + 1);
        let failed = p.env.send_err(&[ix], &[signer]);
        assert_anchor_error(
            &failed,
            anchor_lang::error::ErrorCode::AccountNotInitialized,
        );
    }
    assert_eq!(
        current_key_of(&p, &p.nodes[0].pda),
        (p.nodes[0].key.x, p.nodes[0].key.y)
    );
}

#[test]
fn a_rotation_needs_the_current_keys_signature() {
    let mut p = protocol("rotate-old-sig");
    let new_key = fresh_key("rotate-old-sig-new");
    let version = p.current_version();
    let node = p.nodes[1].pda;
    let owner = p.nodes[1].owner.insecure_clone();

    let mut cases = Vec::new();
    // Signed by some other key, as an operator who lost the node's key would have to.
    let imposter = fresh_key("rotate-old-sig-imposter");
    cases.push(crypto::key_rotation_proof(
        &molpha::ID,
        &node,
        &imposter,
        &new_key,
    ));
    // The new key authorizing itself.
    cases.push(crypto::key_rotation_proof(
        &molpha::ID,
        &node,
        &new_key,
        &new_key,
    ));
    // A damaged signature.
    let mut bad_s = p.key_rotation_proof(1, &new_key);
    bad_s.old_key_sig_s[31] ^= 1;
    cases.push(bad_s);
    let mut bad_commitment = p.key_rotation_proof(1, &new_key);
    bad_commitment.old_key_commitment[0] ^= 1;
    cases.push(bad_commitment);

    for proof in cases {
        let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), proof, version + 1);
        let failed = p.env.send_err(&[ix], &[&owner]);
        assert_molpha_error(&failed, MolphaError::InvalidKeyRotationSignature);
    }
    assert_eq!(p.current_version(), version);
    assert!(!p
        .env
        .account_exists(&pda::secp256k1_key_claim(&new_key.compressed)));
}

#[test]
fn a_rotation_needs_the_new_keys_proof_of_possession() {
    let mut p = protocol("rotate-pop");
    let version = p.current_version();
    let node = p.nodes[1].pda;
    let old = p.nodes[1].key.clone();
    let owner = p.nodes[1].owner.insecure_clone();
    let new_key = fresh_key("rotate-pop-new");

    // The proof of possession is made by the old key, not by the key being installed.
    let by_old_key =
        crypto::key_rotation_proof_with(&molpha::ID, &node, &old, &new_key, &old.secret);
    let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), by_old_key, version + 1);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    // A rogue key: the sum of an attacker-held key and the negation of an honest node's key, so
    // that the pair sums to the attacker's key alone. Its owner knows no secret for it.
    let attacker = fresh_key("rotate-pop-attacker");
    let honest = &p.nodes[3].key;
    let mut negated = honest.compressed;
    negated[0] ^= 1; // 0x02 <-> 0x03: the same x, the other y, which is the point's negation.
    let rogue = PublicKey::combine(&[
        attacker.public_key(),
        PublicKey::parse_compressed(&negated).unwrap(),
    ])
    .unwrap();
    let uncompressed = rogue.serialize();
    let rogue_key = NodeKey {
        secret: attacker.secret,
        compressed: rogue.serialize_compressed(),
        x: uncompressed[1..33].try_into().unwrap(),
        y: uncompressed[33..65].try_into().unwrap(),
    };
    let proof =
        crypto::key_rotation_proof_with(&molpha::ID, &node, &old, &rogue_key, &attacker.secret);
    let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), proof, version + 1);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    // A key that is not a point at all.
    let mut malformed = p.key_rotation_proof(1, &new_key);
    malformed.new_public_key_compressed = [0u8; 33];
    let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), malformed, version + 1);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::PubkeyNotOnCurve);

    assert_eq!(p.current_version(), version);
    assert_eq!(snapshot_key(&p, version, &node), (old.x, old.y));
}

#[test]
fn a_proof_made_for_another_node_is_rejected() {
    let mut p = protocol("rotate-other-node");
    let new_key = fresh_key("rotate-other-node-new");
    let version = p.current_version();
    // Everything is valid for node 0, but node 1's owner submits it.
    let proof = p.key_rotation_proof(0, &new_key);
    let owner = p.nodes[1].owner.insecure_clone();
    let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), proof, version + 1);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidPoP);
    assert_eq!(p.current_version(), version);
}

#[test]
fn a_key_can_only_ever_back_one_node() {
    let mut p = protocol("rotate-claims");
    let version = p.current_version();
    let key_a = p.nodes[0].key.clone();
    let key_b = fresh_key("rotate-claims-b");

    // Another node's current key.
    let proof = p.key_rotation_proof(1, &p.nodes[2].key.clone());
    let ix = p.rotate_node_key_ix(1, proof);
    let owner = p.nodes[1].owner.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));

    // Its own current key, which would only burn a registry version.
    let proof = p.key_rotation_proof(1, &p.nodes[1].key.clone());
    let ix = p.rotate_node_key_ix(1, proof);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));
    assert_eq!(p.current_version(), version);

    // A key retired by a rotation stays reserved, for its own node and for every other.
    p.rotate_node_key(0, key_b.clone());
    p.env.advance_seconds(WINDOW_SECONDS);
    let owner0 = p.nodes[0].owner.insecure_clone();
    let back = p.key_rotation_proof(0, &key_a);
    let ix = p.rotate_node_key_ix(0, back);
    let failed = p.env.send_err(&[ix], &[&owner0]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));
    let to_retired = p.key_rotation_proof(1, &key_a);
    let ix = p.rotate_node_key_ix(1, to_retired);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));
    assert_eq!(p.current_version(), version + 1);
}

#[test]
fn a_node_rotates_at_most_once_per_settlement_window() {
    let mut p = protocol("rotate-cooldown");
    let owner = p.nodes[0].owner.insecure_clone();
    let first = fresh_key("rotate-cooldown-1");
    let second = fresh_key("rotate-cooldown-2");
    let third = fresh_key("rotate-cooldown-3");

    p.rotate_node_key(0, first);
    let rotated_at = p.env.unix_timestamp();
    let version = p.current_version();

    // Immediately, and a second before the window closes.
    for wait in [0, WINDOW_SECONDS - 1] {
        p.env.warp_to(rotated_at + wait);
        let proof = p.key_rotation_proof(0, &second);
        let ix = p.rotate_node_key_ix(0, proof);
        let failed = p.env.send_err(&[ix], &[&owner]);
        assert_molpha_error(&failed, MolphaError::KeyRotationCooldownActive);
        assert_eq!(p.current_version(), version);
    }

    // When it closes, and another node is never held back by this one's cooldown.
    p.env.warp_to(rotated_at + WINDOW_SECONDS);
    p.rotate_node_key(0, second);
    assert_eq!(p.current_version(), version + 1);
    p.rotate_node_key(1, third);
    assert_eq!(p.current_version(), version + 2);
}

#[test]
fn only_an_active_node_can_rotate() {
    let mut p = protocol("rotate-active");
    let index = 3;
    let owner = p.nodes[index].owner.insecure_clone();
    let new_key = fresh_key("rotate-active-new");

    let deactivate = p.deactivate_node_ix(index, &owner.pubkey());
    p.env.send_ok(&[deactivate], &[&owner]);
    let version = p.current_version();

    let proof = p.key_rotation_proof(index, &new_key);
    let ix = p.rotate_node_key_ix(index, proof);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NodeNotActive);
    assert_eq!(p.current_version(), version);
}

#[test]
fn a_rotation_must_target_the_next_registry_version() {
    let mut p = protocol("rotate-version");
    let owner = p.nodes[0].owner.insecure_clone();
    let proof = p.key_rotation_proof(0, &fresh_key("rotate-version-new"));
    let ix = p.rotate_node_key_raw_ix(&owner.pubkey(), proof, p.current_version() + 2);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

#[test]
fn rotations_of_different_nodes_compose() {
    let mut p = protocol("rotate-compose");
    let start = p.current_version();
    let (key_one, key_two) = (fresh_key("rotate-compose-1"), fresh_key("rotate-compose-2"));
    p.rotate_node_key(1, key_one.clone());
    p.rotate_node_key(4, key_two.clone());

    // Each version differs from its predecessor in exactly one member.
    let (v0, v1, v2) = (
        p.env.registry(start),
        p.env.registry(start + 1),
        p.env.registry(start + 2),
    );
    assert_eq!(v1.keys[1], (key_one.x, key_one.y));
    assert_eq!(v1.keys[4], v0.keys[4]);
    assert_eq!(v2.keys[1], (key_one.x, key_one.y));
    assert_eq!(v2.keys[4], (key_two.x, key_two.y));
    for bit in [0, 2, 3, 5] {
        assert_eq!(v2.keys[bit], v0.keys[bit]);
    }
    assert_eq!(v0.keys[1], snapshot_key(&p, start, &p.nodes[1].pda));
}

// ---------------------------------------------------------------------------
// history: nothing signed under an earlier version stops verifying
// ---------------------------------------------------------------------------

#[test]
fn an_attestation_signed_before_a_rotation_still_verifies_against_its_own_version() {
    let mut p = protocol("rotate-old-attestation");
    let node = p.nodes[0].pda;
    let duty = duty_selecting(&p, &node);
    let signed = p.sign_duty(duty);
    let old_version = duty.registry_version;

    p.rotate_node_key(0, fresh_key("rotate-old-attestation-new"));
    assert_eq!(p.current_version(), old_version + 1);

    // The Node account no longer holds the key that signed, the snapshot does.
    assert_ne!(
        current_key_of(&p, &node),
        snapshot_key(&p, old_version, &node)
    );
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);
}

#[test]
fn each_version_accepts_only_the_key_it_names() {
    let mut p = protocol("rotate-each-version");
    let node = p.nodes[0].pda;
    let old = p.nodes[0].key.clone();
    let old_duty = duty_selecting(&p, &node);
    let old_version = old_duty.registry_version;

    let new = fresh_key("rotate-each-version-new");
    p.rotate_node_key(0, new.clone());
    let new_duty = duty_selecting(&p, &node);
    assert_eq!(new_duty.registry_version, old_version + 1);

    // Signed with the key its own version names: accepted.
    let submitter = p.env.keypair();
    let on_new = p.sign_duty(new_duty);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &on_new);
    p.env.send_ok(&[ix], &[&submitter]);

    // The retired key can no longer sign for the new version…
    let other = p.env.keypair();
    let stale_key = signed_with_substituted_key(&p, &on_new, &node, &old);
    let ix = p.submit_attestation_ix(&other.pubkey(), &stale_key);
    let failed = p.env.send_err(&[ix], &[&other]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    // …and the new key could not have signed for the version before it.
    let on_old = p.sign_duty_with_bits(old_duty, &p.selection_bits(&old_duty), None);
    let premature = signed_with_substituted_key(&p, &on_old, &node, &new);
    let ix = p.submit_attestation_ix(&other.pubkey(), &premature);
    let failed = p.env.send_err(&[ix], &[&other]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    // The honest old-version attestation is still good while its version is in grace.
    let ix = p.submit_attestation_ix(&other.pubkey(), &on_old);
    p.env.send_ok(&[ix], &[&other]);
}

#[test]
fn a_ticket_for_a_round_before_a_rotation_is_accepted_after_it() {
    let mut p = protocol("rotate-ticket");
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let duty = p.live_duty(3);
    let signed = p.sign_canonical(duty);
    let signer = p
        .nodes
        .iter()
        .position(|n| signed.signer_pdas.contains(&n.pda))
        .expect("a signer");

    p.rotate_node_key(signer, fresh_key("rotate-ticket-new"));

    // The round belongs to the version it ran under, which was live when it did.
    p.submit_ticket(&submitter, &gateway, &signed);
    assert_eq!(p.env.epoch(0).tickets, 1);

    // And a round under the new version counts too.
    let next = p.live_duty(3).source_id([0x77; 32]);
    assert_eq!(next.registry_version, duty.registry_version + 1);
    p.submit_ticket(&submitter, &gateway, &p.sign_canonical(next));
    assert_eq!(p.env.epoch(0).tickets, 2);
}

/// Equivocation evidence names a registry version, so the node it convicts is the one that signed
/// under that version, whatever key it holds by the time the filing lands.
fn equivocation_after_rotations(rotations: usize) {
    let mut p = Protocol::start();
    p.register_nodes("rotate-equivocation", 8);
    let reporter = p.reporter();
    let duty = Duty::new(8, SIGS);
    let selection = p.selection_bits(&duty);
    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[2..6], None);
    let guilty_pda = p.env.registry(duty.registry_version).nodes[selection[2]];
    let guilty_index = index_of(&p, &guilty_pda);

    for round in 0..rotations {
        if round > 0 {
            p.env.advance_seconds(WINDOW_SECONDS);
        }
        p.rotate_node_key(
            guilty_index,
            fresh_key(&format!("rotate-equivocation-{round}")),
        );
    }
    assert_ne!(
        current_key_of(&p, &guilty_pda),
        snapshot_key(&p, duty.registry_version, &guilty_pda),
        "the guilty node no longer holds the key it signed with"
    );

    let evidence = EquivocationEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    };
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    p.env.send_ok(&[ix], &[&reporter]);

    let node: Node = p.env.get(&guilty_pda);
    assert_eq!(node.status, NodeStatus::Tombstoned);
    assert!(!p
        .env
        .registry(p.current_version())
        .nodes
        .contains(&guilty_pda));
}

#[test]
fn equivocation_evidence_convicts_a_node_that_has_rotated_once() {
    equivocation_after_rotations(1);
}

#[test]
fn equivocation_evidence_convicts_a_node_that_has_rotated_twice() {
    equivocation_after_rotations(2);
}

/// Equivocation by a node that then rotates away from the key it signed with, and the evidence.
fn rotated_equivocation(domain: &str) -> (Protocol, Keypair, Pubkey, u32, EquivocationEvidence) {
    let mut p = Protocol::start();
    p.register_nodes(domain, 8);
    let reporter = p.reporter();
    let duty = Duty::new(8, SIGS);
    let selection = p.selection_bits(&duty);
    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[2..6], None);
    let guilty_pda = p.env.registry(duty.registry_version).nodes[selection[2]];
    let guilty_index = index_of(&p, &guilty_pda);
    p.rotate_node_key(guilty_index, fresh_key(&format!("{domain}-new")));
    let evidence = EquivocationEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    };
    (p, reporter, guilty_pda, duty.registry_version, evidence)
}

/// Evidence against a superseded version can be filed until one settlement window after the
/// version's grace ends, and not after. So a key a node rotated away from, perhaps because it
/// leaked, cannot be used to frame the node for longer than that.
#[test]
fn evidence_against_a_superseded_version_can_be_filed_for_one_window_after_its_grace() {
    let (mut p, reporter, guilty, version, evidence) = rotated_equivocation("evidence-open");
    let grace = p.env.registry(version).grace_active_until;
    assert!(grace > 0, "the rotation superseded the evidence's version");

    p.env.warp_to(grace + WINDOW_SECONDS - 1);
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    p.env.send_ok(&[ix], &[&reporter]);
    assert_eq!(p.env.get::<Node>(&guilty).status, NodeStatus::Tombstoned);
}

#[test]
fn evidence_against_a_superseded_version_expires_one_window_after_its_grace() {
    let (mut p, reporter, guilty, version, evidence) = rotated_equivocation("evidence-expired");
    let grace = p.env.registry(version).grace_active_until;

    p.env.warp_to(grace + WINDOW_SECONDS);
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::EvidenceExpired);
    assert_eq!(p.env.get::<Node>(&guilty).status, NodeStatus::Active);
}

#[test]
fn a_frozen_node_rejoins_with_the_key_it_holds_now() {
    let mut p = Protocol::builder().min_availability_samples(3).start();
    p.register_nodes("rotate-reinstate", 4);
    let gateway = p.register_gateway();
    let index = 1;
    let node = p.nodes[index].pda;
    let owner = p.nodes[index].owner.pubkey();
    let new_key = fresh_key("rotate-reinstate-new");

    p.rotate_node_key(index, new_key.clone());
    p.freeze_via_inactivity(&gateway, &node);
    assert_eq!(p.env.get::<Node>(&node).status, NodeStatus::Frozen);

    p.env.advance_seconds(86_400 + 1);
    let stranger = p.env.keypair();
    let ix = p.reinstate_node_ix(&stranger.pubkey(), &owner);
    p.env.send_ok(&[ix], &[&stranger]);

    let current = p.env.registry(p.current_version());
    assert_eq!(
        snapshot_key(&p, p.current_version(), &node),
        (new_key.x, new_key.y)
    );
    assert_eq!(current.node_count, 4);
}

// ---------------------------------------------------------------------------
// cost and capacity
// ---------------------------------------------------------------------------

/// A rotation copies a snapshot, creates it and a key claim, and verifies two signatures: the
/// current key's and the new key's proof of possession. `deactivate_node` pays the same snapshot
/// cost with no signatures (about 18k), and `register_node` pays the same proof of possession, so
/// a rotation lands near a registration: the proof of possession is almost all of it.
const ROTATE_CU_CEILING: u64 = 480_000;

#[test]
fn a_rotation_stays_inside_its_compute_ceiling() {
    let mut p = protocol("rotate-cu");
    let owner = p.nodes[1].owner.insecure_clone();
    let deactivate = p.deactivate_node_ix(1, &owner.pubkey());
    let deactivation = p.env.send_ok(&[deactivate], &[&owner]);

    let newcomer = fresh_key("rotate-cu-newcomer");
    let owner: Keypair = p.env.keypair();
    let deposit = p.args.minimum_node_deposit;
    p.fund_usdc(&owner.pubkey(), deposit * 4);
    let args = p.register_node_args(&owner.pubkey(), &newcomer, [0xcc; 32], 9_100);
    let authority = p.authority.insecure_clone();
    let ix = p.register_node_ix(&owner.pubkey(), &args, &authority.pubkey());
    let registration = p.env.send_ok(&[ix], &[&authority, &owner]);

    let rotation = p.rotate_node_key(0, fresh_key("rotate-cu-new"));
    println!(
        "ROTATE_NODE_KEY cu={} register_node cu={} deactivate_node cu={}",
        rotation.compute_units_consumed,
        registration.compute_units_consumed,
        deactivation.compute_units_consumed
    );
    assert!(rotation.compute_units_consumed < ROTATE_CU_CEILING);
}
