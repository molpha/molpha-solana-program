//! How registry snapshot accounts are allocated.
//!
//! A snapshot is exactly as large as its slots need (a removed member leaves a hole, and a hole in
//! the middle keeps its record), so the only limit on membership is the width of a signer bitmap:
//! 256. One instruction can create or grow an account by at most 10 KiB,
//! which is a snapshot of 106 members; past that, `extend_registry` allocates the next version
//! ahead of the instruction that publishes it.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::system_instruction;
use molpha::error::MolphaError;
use molpha::events::RegistryExtended;
use molpha::state::{EquivocationEvidence, Node, NodeStatus, Registry, MAX_REGISTRY_NODES};
use molpha::utils::registry_account::MAX_GROWTH_PER_INSTRUCTION;
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event, PACKET_DATA_SIZE};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

/// The largest snapshot one instruction can create.
const SINGLE_INSTRUCTION_MEMBERS: usize = 106;
const MAX_COMPUTE_UNITS: u64 = 1_400_000;

fn protocol(domain: &str, count: usize) -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes(domain, count);
    p
}

fn next_registry(p: &Protocol) -> Pubkey {
    pda::registry(p.current_version() + 1)
}

fn fresh_key(domain: &str) -> NodeKey {
    crypto::node_keys(domain, 1).remove(0)
}

/// The current snapshot holds exactly its members, and exactly the rent that takes.
fn assert_exact_fit(p: &Protocol) -> usize {
    let version = p.current_version();
    // `registry` itself asserts the account is `Registry::space(node_count)` long.
    let registry = p.env.registry(version);
    assert_eq!(
        p.env.lamports(&pda::registry(version)),
        p.env.rent_minimum(registry.data_len),
        "version {version} holds more or less than its rent"
    );
    registry.data_len
}

/// A funded owner and the instruction that registers a new node for it.
fn registration(p: &mut Protocol, domain: &str) -> (Keypair, Instruction) {
    let key = fresh_key(domain);
    let owner = p.env.keypair();
    let deposit = p.args.minimum_node_deposit;
    p.fund_usdc(&owner.pubkey(), deposit * 4);
    let args = p.register_node_args(&owner.pubkey(), &key, [0xa7; 32], 9_500);
    let ix = p.register_node_ix(&owner.pubkey(), &args, &p.authority.pubkey());
    (owner, ix)
}

fn register(
    p: &mut Protocol,
    owner: &Keypair,
    ixs: &[Instruction],
) -> litesvm::types::TransactionResult {
    let authority = p.authority.insecure_clone();
    p.env.send(ixs, &[&authority, owner])
}

// ---------------------------------------------------------------------------
// exact fit
// ---------------------------------------------------------------------------

#[test]
fn every_version_is_sized_and_funded_for_exactly_its_members() {
    let mut p = Protocol::start();
    assert_eq!(p.env.registry(0).data_len, Registry::space(0));
    assert_eq!(assert_exact_fit(&p), 32);

    for count in 1..=4 {
        p.register_nodes(&format!("exact-fit-{count}"), 1);
        assert_eq!(assert_exact_fit(&p), Registry::space(count));
    }

    // A rotation and a policy change keep the membership, so the size.
    p.rotate_node_key(1, fresh_key("exact-fit-rotated"));
    assert_eq!(assert_exact_fit(&p), Registry::space(4));
    let authority = p.authority.insecure_clone();
    let ix = p.update_registry_policy_ix(&authority.pubkey(), p.current_version() + 1, 3);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(assert_exact_fit(&p), Registry::space(4));

    // A removal in the middle leaves a hole: the slots, and so the size, stay.
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(assert_exact_fit(&p), Registry::space(4));
    assert_eq!(
        p.env.registry(p.current_version()).live_slots(),
        vec![1, 2, 3]
    );

    // Removing the last member trims the slot count, and the next version shrinks.
    let owner = p.nodes[3].owner.insecure_clone();
    let ix = p.deactivate_node_ix(3, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
    assert_eq!(assert_exact_fit(&p), Registry::space(3));

    // A new registration takes the hole at slot 0, so the size does not grow.
    p.register_nodes("exact-fit-refill", 1);
    assert_eq!(assert_exact_fit(&p), Registry::space(3));
    assert_eq!(
        p.env.registry(p.current_version()).live_slots(),
        vec![0, 1, 2]
    );

    // Versions already published keep the size they were given.
    assert_eq!(p.env.registry(1).data_len, Registry::space(1));
    assert_eq!(p.env.registry(4).data_len, Registry::space(4));
}

#[test]
fn a_tail_removal_pays_rent_for_the_smaller_snapshot_only() {
    let mut p = protocol("removal-rent", 5);
    let owner = p.nodes[4].owner.insecure_clone();
    let before = p.env.lamports(&owner.pubkey());

    let ix = p.deactivate_node_ix(4, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);

    // The handler works in a table with room for all five, then shrinks it to four and returns
    // the difference. The owner pays rent only; the transaction fee is the environment payer's.
    assert_eq!(
        before - p.env.lamports(&owner.pubkey()),
        p.env.rent_minimum(Registry::space(4))
    );
    assert_exact_fit(&p);
}

#[test]
fn a_removal_in_the_middle_keeps_every_slot_and_so_the_size() {
    let mut p = protocol("removal-hole-rent", 5);
    let owner = p.nodes[2].owner.insecure_clone();
    let before = p.env.lamports(&owner.pubkey());

    let ix = p.deactivate_node_ix(2, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);

    // Slot 2 is a hole inside a table of five, so the next version is five records long.
    assert_eq!(
        before - p.env.lamports(&owner.pubkey()),
        p.env.rent_minimum(Registry::space(5))
    );
    assert_eq!(assert_exact_fit(&p), Registry::space(5));
    assert_eq!(
        p.env.registry(p.current_version()).live_slots(),
        vec![0, 1, 3, 4]
    );
}

// ---------------------------------------------------------------------------
// extend_registry
// ---------------------------------------------------------------------------

#[test]
fn anyone_can_allocate_the_next_version_and_only_up_to_what_it_could_need() {
    let mut p = protocol("extend-small", 5);
    let stranger = p.env.keypair();
    let next = next_registry(&p);
    assert!(!p.env.account_exists(&next));

    let ix = p.extend_registry_ix(&stranger.pubkey());
    let meta = p.env.send_ok(&[ix], &[&stranger]);

    // One member more than the current version is the most the next one could hold.
    let target = Registry::space(6);
    let data = p.env.account_data(&next);
    assert_eq!(data.len(), target);
    assert!(data.iter().all(|byte| *byte == 0), "allocated, not written");
    assert_eq!(p.env.lamports(&next), p.env.rent_minimum(target));
    let event: RegistryExtended = only_event(&meta);
    assert_eq!(event.version, p.current_version() + 1);
    assert_eq!(event.data_len as usize, target);
    assert_eq!(event.target_len as usize, target);

    // Nothing was published.
    assert_eq!(p.current_version(), 5);

    // Extending again changes nothing and costs no rent.
    let balance = p.env.lamports(&stranger.pubkey());
    let ix = p.extend_registry_ix(&stranger.pubkey());
    p.env.send_ok(&[ix], &[&stranger]);
    assert_eq!(p.env.account_data(&next).len(), target);
    assert_eq!(p.env.lamports(&stranger.pubkey()), balance);
}

#[test]
fn a_version_someone_else_allocated_is_published_at_its_exact_size() {
    let mut p = protocol("extend-then-publish", 5);
    let stranger = p.env.keypair();
    let ix = p.extend_registry_ix(&stranger.pubkey());
    p.env.send_ok(&[ix], &[&stranger]);
    let allocated = next_registry(&p);
    assert_eq!(p.env.account_data(&allocated).len(), Registry::space(6));

    // A rotation needs room for five. The pre-allocated account is used, shrunk, and the rent
    // for the spare slot goes to whoever publishes.
    let owner = p.nodes[0].owner.pubkey();
    let before = p.env.lamports(&owner);
    p.rotate_node_key(0, fresh_key("extend-then-publish-new"));

    assert_eq!(pda::registry(p.current_version()), allocated);
    assert_eq!(assert_exact_fit(&p), Registry::space(5));
    let spare = p.env.rent_minimum(Registry::space(6)) - p.env.rent_minimum(Registry::space(5));
    let claim = p.env.rent_minimum(8 + 33);
    assert_eq!(
        p.env.lamports(&owner) as i128 - before as i128,
        spare as i128 - claim as i128,
        "the publisher pays for the key claim and is refunded the spare slot"
    );
}

#[test]
fn extend_registry_only_reaches_the_next_version() {
    let mut p = protocol("extend-target", 3);
    let payer = p.env.keypair();

    // A published version is not the next version's PDA.
    let mut ix = p.extend_registry_ix(&payer.pubkey());
    ix.accounts[3].pubkey = pda::registry(p.current_version());
    let failed = p.env.send_err(&[ix], &[&payer]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);

    // Nor is a version further ahead.
    let mut ix = p.extend_registry_ix(&payer.pubkey());
    ix.accounts[3].pubkey = pda::registry(p.current_version() + 2);
    let failed = p.env.send_err(&[ix], &[&payer]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);

    assert_eq!(
        p.env.registry(p.current_version()).data_len,
        Registry::space(3)
    );
}

/// Anyone can send lamports to an address, and `create_account` refuses an address that holds
/// any. The next version must not be blockable that way.
#[test]
fn lamports_sent_to_the_next_version_ahead_of_time_do_not_block_it() {
    let mut p = protocol("prefunded", 3);
    let griefer = p.env.keypair();

    // The least the runtime lets an address hold, which is less than a snapshot's rent: the
    // publisher tops it up.
    let dust = p.env.rent_minimum(0);
    assert!(dust < p.env.rent_minimum(Registry::space(4)));
    let ix = system_instruction::transfer(&griefer.pubkey(), &next_registry(&p), dust);
    p.env.send_ok(&[ix], &[&griefer]);
    p.register_nodes("prefunded-a", 1);
    assert_eq!(assert_exact_fit(&p), Registry::space(4));

    // Far more than rent: the snapshot keeps what it needs and the publisher gets the rest.
    let gift = 5_000_000_000;
    let ix = system_instruction::transfer(&griefer.pubkey(), &next_registry(&p), gift);
    p.env.send_ok(&[ix], &[&griefer]);
    let owner = p.nodes[0].owner.insecure_clone();
    let before = p.env.lamports(&owner.pubkey());
    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
    // Slot 0 becomes a hole inside four slots, so the version keeps four records.
    assert_eq!(assert_exact_fit(&p), Registry::space(4));
    assert_eq!(
        p.env.lamports(&owner.pubkey()) - before,
        gift - p.env.rent_minimum(Registry::space(4))
    );

    // And `extend_registry` takes over a funded address too.
    let ix = system_instruction::transfer(&griefer.pubkey(), &next_registry(&p), dust);
    p.env.send_ok(&[ix], &[&griefer]);
    let ix = p.extend_registry_ix(&griefer.pubkey());
    p.env.send_ok(&[ix], &[&griefer]);
    // It grows toward one slot more than the current version has, the most the next could need;
    // the registration then takes the hole and gives the spare record back.
    assert_eq!(
        p.env.account_data(&next_registry(&p)).len(),
        Registry::space(5)
    );
    p.register_nodes("prefunded-b", 1);
    assert_eq!(assert_exact_fit(&p), Registry::space(4));
}

// ---------------------------------------------------------------------------
// past one instruction's reach
// ---------------------------------------------------------------------------

#[test]
fn a_snapshot_past_106_members_needs_the_next_version_extended_first() {
    let mut p = protocol("past-106", SINGLE_INSTRUCTION_MEMBERS);
    assert_eq!(assert_exact_fit(&p), 10_208);
    assert!(Registry::space(SINGLE_INSTRUCTION_MEMBERS) <= MAX_GROWTH_PER_INSTRUCTION);
    assert!(Registry::space(SINGLE_INSTRUCTION_MEMBERS + 1) > MAX_GROWTH_PER_INSTRUCTION);
    assert_eq!(p.registry_extends_needed(SINGLE_INSTRUCTION_MEMBERS), 0);
    assert_eq!(p.registry_extends_needed(SINGLE_INSTRUCTION_MEMBERS + 1), 1);

    // The 107th member does not fit an account one instruction can create.
    let (owner, register_ix) = registration(&mut p, "past-106-next");
    let failed = register(&mut p, &owner, std::slice::from_ref(&register_ix)).unwrap_err();
    assert_molpha_error(&failed, MolphaError::RegistryNotAllocated);
    assert_eq!(p.current_version() as usize, SINGLE_INSTRUCTION_MEMBERS);

    // One `extend_registry` in front of it does: it allocates 10 KiB, and the registration adds
    // the rest.
    let extend = p.extend_registry_ix(&owner.pubkey());
    register(&mut p, &owner, &[extend, register_ix]).expect("extended registration");
    assert_eq!(assert_exact_fit(&p), Registry::space(107));
    assert_eq!(p.env.registry(p.current_version()).node_count, 107);

    // Every later version of this size needs the same: a rotation…
    let rotating = p.nodes[3].owner.insecure_clone();
    let proof = p.key_rotation_proof(3, &fresh_key("past-106-rotated"));
    let rotate = p.rotate_node_key_ix(3, proof);
    let failed = p.env.send_err(std::slice::from_ref(&rotate), &[&rotating]);
    assert_molpha_error(&failed, MolphaError::RegistryNotAllocated);
    let extend = p.extend_registry_ix(&rotating.pubkey());
    p.env.send_ok(&[extend, rotate], &[&rotating]);
    assert_eq!(assert_exact_fit(&p), Registry::space(107));

    // …and a removal, which copies all 107 before it drops one. Removing the last member ends at
    // a size one instruction could have created, so the version after it needs no extending
    // again; a removal in the middle would leave a hole and keep all 107 slots.
    // The 107th member, registered above, holds the last slot.
    let leaving = owner.insecure_clone();
    let deactivate = p.deactivate_node_raw_ix(&leaving.pubkey(), &leaving.pubkey());
    let failed = p
        .env
        .send_err(std::slice::from_ref(&deactivate), &[&leaving]);
    assert_molpha_error(&failed, MolphaError::RegistryNotAllocated);
    let extend = p.extend_registry_ix(&leaving.pubkey());
    p.env.send_ok(&[extend, deactivate], &[&leaving]);
    assert_eq!(assert_exact_fit(&p), Registry::space(106));
    assert_eq!(p.registry_extends_needed(106), 0);
}

/// A registry filled to the width of the signer bitmap.
fn full_registry(domain: &str) -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes(&format!("{domain}-a"), SINGLE_INSTRUCTION_MEMBERS);
    // Up to 213 members the publishing instruction needs one extension in front of it; from 214,
    // two. `register_nodes` adds them.
    assert_eq!(p.registry_extends_needed(107), 1);
    p.register_nodes(&format!("{domain}-b"), 213 - SINGLE_INSTRUCTION_MEMBERS);
    assert_eq!(p.registry_extends_needed(213), 1);
    assert_eq!(p.registry_extends_needed(214), 2);
    p.register_nodes(&format!("{domain}-c"), MAX_REGISTRY_NODES - 213);
    assert_eq!(p.registry_extends_needed(MAX_REGISTRY_NODES), 2);
    assert_eq!(p.current_members(), MAX_REGISTRY_NODES);
    p
}

#[test]
fn a_registry_fills_to_256_members_and_every_transition_still_works() {
    let mut p = full_registry("full");
    assert_eq!(assert_exact_fit(&p), 24_608);
    let version = p.current_version();

    // A 257th member has no bit in a signer bitmap.
    let (owner, register_ix) = registration(&mut p, "full-extra");
    let mut ixs = p.registry_extend_ixs(&owner.pubkey(), MAX_REGISTRY_NODES);
    ixs.push(register_ix);
    let failed = register(&mut p, &owner, &ixs).unwrap_err();
    assert_molpha_error(&failed, MolphaError::NodeCapExceeded);
    assert_eq!(p.current_version(), version);

    // A rotation of the last member, at the far end of the table.
    let last = MAX_REGISTRY_NODES - 1;
    let new_key = fresh_key("full-rotated");
    let meta = p.rotate_node_key(last, new_key.clone());
    println!(
        "FULL_REGISTRY rotate_node_key cu={}",
        meta.compute_units_consumed
    );
    assert_eq!(assert_exact_fit(&p), 24_608);
    assert_eq!(
        p.env.registry(p.current_version()).keys[last],
        (new_key.x, new_key.y)
    );

    // An attestation is verified against the 24 KiB snapshot with no signer accounts.
    let submitter = p.env.keypair();
    let duty = Duty::new(p.current_version(), 18);
    let signed = p.sign_duty(duty);
    assert_eq!(signed.signer_pdas.len(), 20);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let meta = p.env.send_ok(&[ix], &[&submitter]);
    println!(
        "FULL_REGISTRY submit_attestation signers=20 cu={}",
        meta.compute_units_consumed
    );

    // Slashing evicts the guilty into a smaller snapshot. The reporter extends the next version
    // for the copy the handler makes before it removes them.
    let reporter = p.reporter();
    let duty = Duty::new(p.current_version(), 4).source_id([0x5a; 32]);
    let selection = p.selection_bits(&duty);
    let a = p.sign_duty_with_bits(duty.value([0x01; 32]), &selection[0..4], None);
    let b = p.sign_duty_with_bits(duty.value([0x02; 32]), &selection[2..6], None);
    let guilty: Vec<Pubkey> = [selection[2], selection[3]]
        .iter()
        .map(|bit| p.env.registry(duty.registry_version).nodes[*bit])
        .collect();
    let mut ixs = p.registry_extend_ixs(&reporter.pubkey(), MAX_REGISTRY_NODES);
    assert_eq!(ixs.len(), 2);
    ixs.push(p.slash_equivocation_ix(
        &reporter.pubkey(),
        EquivocationEvidence {
            attestation_a: a.attestation(),
            attestation_b: b.attestation(),
            coalition_key_a: a.coalition_key,
            coalition_key_b: b.coalition_key,
        },
    ));
    let meta = p.env.send_ok(&ixs, &[&reporter]);
    println!(
        "FULL_REGISTRY slash_offence cu={}",
        meta.compute_units_consumed
    );
    // The guilty leave holes: the table keeps its 256 records unless one of them was the last.
    let after = p.env.registry(p.current_version());
    assert_eq!(after.live_slots().len(), MAX_REGISTRY_NODES - 2);
    assert_eq!(
        assert_exact_fit(&p),
        Registry::space(usize::from(after.node_count))
    );
    for node in &guilty {
        assert_eq!(p.env.get::<Node>(node).status, NodeStatus::Tombstoned);
        assert!(!after.nodes.contains(node));
    }

    // With two holes free the registry takes members again, into the holes.
    p.register_nodes("full-refill", 2);
    let refilled = p.env.registry(p.current_version());
    assert_eq!(refilled.live_slots().len(), MAX_REGISTRY_NODES);
    assert_eq!(assert_exact_fit(&p), 24_608);
}

/// Lifting the member limit does not lift the coalition limit: verifying an aggregate costs
/// compute per signer, and one transaction has 1.4M units.
#[test]
fn a_coalition_is_bounded_by_compute_not_by_the_registry() {
    let mut p = Protocol::builder().redundancy_buffer(2).start();
    p.register_nodes("coalition-a", SINGLE_INSTRUCTION_MEMBERS);
    p.register_nodes("coalition-b", 180 - SINGLE_INSTRUCTION_MEMBERS);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();

    // 150 signers fit.
    let fits = Duty::new(p.current_version(), 148).at_secs(p.env.unix_timestamp() as u64);
    let signed = p.sign_duty(fits);
    assert_eq!(signed.signer_pdas.len(), 150);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let meta = p.env.send_ok(&[ix], &[&submitter]);
    println!(
        "ATTESTATION_CU coalition=150 cu={}",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);

    // So does its ticket, in a legacy packet.
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let size = p.env.transaction_size_as(&submitter.pubkey(), &ixs);
    println!("TICKET signers=150 size={size}");
    assert!(size <= PACKET_DATA_SIZE);

    // 175 do not, whatever the registry holds.
    let too_many = Duty::new(p.current_version(), 173)
        .source_id([0x33; 32])
        .at_secs(p.env.unix_timestamp() as u64);
    let signed = p.sign_duty(too_many);
    assert_eq!(signed.signer_pdas.len(), 175);
    let other = p.env.keypair();
    let ix = p.submit_attestation_ix(&other.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&other]);
    assert!(
        failed.meta.compute_units_consumed >= MAX_COMPUTE_UNITS - 1_000,
        "expected the compute budget to run out, used {}\n{}",
        failed.meta.compute_units_consumed,
        failed.meta.logs.join("\n")
    );
}
