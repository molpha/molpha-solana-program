//! Stable registry slots and selection-free verification.
//!
//! Removing a node leaves a hole instead of moving the last member into its slot, so every other
//! node keeps the bit it signs with. A registration takes the lowest hole. Consumers verify any
//! quorum of live members; the selection still decides who is asked, who is counted for
//! activity (tickets) and who is slashable (unsolicited signing).
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::state::{Node, NodeStatus, UnsolicitedSigningEvidence};
use molpha_tests::crypto;
use molpha_tests::env::assert_molpha_error;
use molpha_tests::scenario::{Duty, Protocol};
use solana_signer::Signer;

const NODE_COUNT: usize = 8;

fn remove(p: &mut Protocol, helper_index: usize) {
    let owner = p.nodes[helper_index].owner.insecure_clone();
    let ix = p.deactivate_node_ix(helper_index, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
}

fn protocol(domain: &str) -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes(domain, NODE_COUNT);
    p
}

#[test]
fn removed_nodes_leave_holes_and_the_others_keep_their_slots() {
    let mut p = protocol("slots-holes");
    let before = p.env.registry(p.current_version());
    remove(&mut p, 2);
    remove(&mut p, 5);

    let after = p.env.registry(p.current_version());
    assert_eq!(after.node_count as usize, NODE_COUNT);
    assert_eq!(after.live_slots(), vec![0, 1, 3, 4, 6, 7]);
    for slot in after.live_slots() {
        assert_eq!(after.nodes[slot], before.nodes[slot], "slot {slot} moved");
        assert_eq!(
            after.keys[slot], before.keys[slot],
            "slot {slot} changed key"
        );
    }
    assert_eq!(after.nodes[2], Pubkey::default());
    assert_eq!(after.nodes[5], Pubkey::default());
}

#[test]
fn a_registration_takes_the_lowest_hole() {
    let mut p = protocol("slots-reuse");
    remove(&mut p, 5);
    remove(&mut p, 2);

    p.register_nodes("slots-reuse-newcomer", 1);
    let newcomer = p.nodes.last().unwrap();
    assert_eq!(newcomer.index, 2, "the lowest hole");
    let registry = p.env.registry(p.current_version());
    assert_eq!(registry.nodes[2], newcomer.pda);
    assert_eq!(registry.nodes[5], Pubkey::default());
    assert_eq!(registry.node_count as usize, NODE_COUNT);

    p.register_nodes("slots-reuse-second", 1);
    assert_eq!(p.nodes.last().unwrap().index, 5);

    // No hole is left, so the next registration goes after the last slot.
    p.register_nodes("slots-reuse-third", 1);
    assert_eq!(p.nodes.last().unwrap().index as usize, NODE_COUNT);
}

#[test]
fn the_selection_draws_over_live_members_only() {
    let mut p = protocol("slots-selection");
    remove(&mut p, 2);
    remove(&mut p, 6);
    let version = p.current_version();
    let live = p.env.registry(version).live_slots();
    assert_eq!(live.len(), NODE_COUNT - 2);

    for w in 0..16u64 {
        let duty = Duty::new(version, 3).at_secs(1_700_000_000 + w * 7);
        let bits = p.selection_bits(&duty);
        assert_eq!(
            bits.len(),
            5,
            "3 + the default buffer of 2, over 6 live nodes"
        );
        assert!(
            bits.iter().all(|bit| live.contains(bit)),
            "{bits:?} vs {live:?}"
        );
    }
}

#[test]
fn consumers_accept_an_attestation_from_a_registry_with_holes() {
    let mut p = protocol("slots-submit");
    remove(&mut p, 1);
    let version = p.current_version();
    let submitter = p.env.keypair();

    let signed = p.sign_duty(Duty::new(version, 3));
    assert!(!crypto::bitmap_bits(&signed.signers_bitmap).contains(&1));
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);
}

#[test]
fn a_signer_bit_on_a_hole_is_refused() {
    let mut p = protocol("slots-bit-on-hole");
    remove(&mut p, 1);
    let submitter = p.env.keypair();
    let signed = p.sign_duty(Duty::new(p.current_version(), 3));

    // Move one signer's bit onto the hole: same popcount, so only the hole check can refuse it.
    let mut bits = crypto::bitmap_bits(&signed.signers_bitmap);
    bits[0] = 1;
    let mut args = signed.submit_args();
    args.attestation.signature.signers_bitmap = crypto::bitmap_from_bits(&bits);
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidSignersBitmap);
}

#[test]
fn a_ticket_still_requires_the_signers_to_be_selected() {
    let mut p = protocol("slots-ticket");
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();

    let duty = p.live_duty(3);
    let selection = p.selection_bits(&duty);
    let outside = (0..NODE_COUNT)
        .find(|bit| !selection.contains(bit))
        .expect("five of eight are selected");
    let mut bits = selection[..2].to_vec();
    bits.push(outside);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(duty, &bits, None);

    // Consumers would take it, the epoch's activity counters do not.
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);
    assert_eq!(p.env.epoch(0).tickets, 0);
}

#[test]
fn a_ticket_over_a_registry_with_holes_counts_the_live_selection() {
    let mut p = protocol("slots-ticket-holes");
    remove(&mut p, 3);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let removed = p.nodes[3].pda;

    let duty = p.live_duty(3);
    let signed = p.sign_canonical(duty);
    p.submit_ticket(&submitter, &gateway, &signed);

    let epoch = p.env.epoch(0);
    let roster: Vec<[u8; 32]> = epoch.roster[..usize::from(epoch.roster_len)].to_vec();
    assert_eq!(roster.len(), 5);
    assert!(!roster.contains(&removed.to_bytes()));
    assert_eq!(epoch.tickets, 1);
}

#[test]
fn unsolicited_signing_is_still_slashable_over_a_registry_with_holes() {
    let mut p = protocol("slots-unsolicited");
    remove(&mut p, 1);
    let reporter = p.reporter();
    let duty = Duty::new(p.current_version(), 4);
    let selection = p.selection_bits(&duty);
    let live = p.env.registry(duty.registry_version).live_slots();
    let intruder = *live
        .iter()
        .find(|bit| !selection.contains(bit))
        .expect("a live node outside the selection");
    let mut bits = selection[..3].to_vec();
    bits.push(intruder);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(duty, &bits, None);
    let intruder_pda = p.env.registry(duty.registry_version).nodes[intruder];

    // The consumer path accepts the round: the offence is what stands against it.
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);

    let evidence = UnsolicitedSigningEvidence {
        attestation: signed.attestation(),
        coalition_key: signed.coalition_key,
    };
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), evidence);
    p.env.send_ok(&[ix], &[&reporter]);

    let node: Node = p.env.get(&intruder_pda);
    assert_eq!(node.status, NodeStatus::Tombstoned);
    let after = p.env.registry(p.current_version());
    assert!(!after.nodes.contains(&intruder_pda));
    assert_eq!(after.live_slots().len(), NODE_COUNT - 2);
}
