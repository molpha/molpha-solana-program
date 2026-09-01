//! `register_node`, `deactivate_node`, `update_node_endpoint`, `update_registry_policy`, and the
//! immutability of version-addressed registry snapshots.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use molpha::error::MolphaError;
use molpha::events::{
    NodeDeactivated, NodeEndpointUpdated, NodeRegistered, RegistryUpdated, RegistryVersionReason,
};
use molpha::instructions::{DeactivateNodeArgs, RegisterNodeArgs};
use molpha::state::{Node, NodeStatus, RegistryState, Secp256k1KeyClaim};
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn salt(byte: u8) -> [u8; 32] {
    [byte; 32]
}

/// A funded node-owner keypair with a USDC balance large enough for the deposit.
fn funded_owner(p: &mut Protocol) -> Keypair {
    let owner = p.env.keypair();
    let deposit = p.args.minimum_node_deposit;
    p.fund_usdc(&owner.pubkey(), deposit * 4);
    owner
}

fn register(
    p: &mut Protocol,
    owner: &Keypair,
    args: &RegisterNodeArgs,
) -> litesvm::types::TransactionResult {
    let authority = p.authority.insecure_clone();
    let ix = p.register_node_ix(&owner.pubkey(), args, &authority.pubkey());
    p.env.send(&[ix], &[&authority, owner])
}

fn register_ix(
    p: &mut Protocol,
    owner: &Keypair,
    ix: anchor_lang::solana_program::instruction::Instruction,
) -> litesvm::types::TransactionResult {
    let authority = p.authority.insecure_clone();
    p.env.send(&[ix], &[&authority, owner])
}

// ---------------------------------------------------------------------------
// register_node
// ---------------------------------------------------------------------------

#[test]
fn register_node_writes_identity_takes_the_deposit_and_appends_a_snapshot() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-happy", 1).remove(0);
    let owner = funded_owner(&mut p);
    let owner_usdc = pda::ata(&owner.pubkey(), &p.usdc_mint);
    let owner_before = p.env.token_balance(&owner_usdc);

    let args = p.register_node_args(&owner.pubkey(), &key, salt(9), 7_777);
    let authority = p.authority.insecure_clone();
    let ix = p.register_node_ix(&owner.pubkey(), &args, &authority.pubkey());
    let meta = p.env.send_ok(&[ix], &[&authority, &owner]);

    let node: Node = p.env.get(&pda::node(&owner.pubkey()));
    assert_eq!(node.owner, owner.pubkey());
    assert_eq!(node.secp256k1_pubkey_x, key.x);
    assert_eq!(node.secp256k1_pubkey_y, key.y);
    assert_eq!(node.status, NodeStatus::Active);
    assert_eq!(node.ip, [127, 0, 0, 1]);
    assert_eq!(node.port, 7_777);
    assert_eq!(node.locked_amount, p.args.minimum_node_deposit);
    assert_eq!(node.claimable_rewards, 0);
    assert_eq!(node.registered_at, p.env.unix_timestamp());
    assert_eq!(node.withdrawable_slot, 0);
    assert_eq!(node.frozen_until, 0);

    // The secp256k1 key is permanently reserved for this node.
    let claim: Secp256k1KeyClaim = p.env.get(&pda::secp256k1_key_claim(&key.compressed));
    assert_eq!(claim.node_pubkey, pda::node(&owner.pubkey()));

    // Deposit moved owner -> treasury.
    assert_eq!(
        p.env.token_balance(&owner_usdc),
        owner_before - p.args.minimum_node_deposit
    );
    assert_eq!(p.treasury_balance(), p.args.minimum_node_deposit);

    // A fresh snapshot holds the node; the genesis snapshot is untouched.
    assert_eq!(p.current_version(), 1);
    let v1 = p.env.registry(1);
    assert_eq!(v1.version, 1);
    assert_eq!(v1.node_count, 1);
    assert_eq!(v1.nodes, vec![pda::node(&owner.pubkey())]);
    assert_eq!(v1.redundancy_buffer, p.args.redundancy_buffer);
    let v0 = p.env.registry(0);
    assert_eq!(v0.node_count, 0);

    let registered: NodeRegistered = only_event(&meta);
    assert_eq!(registered.owner, owner.pubkey());
    assert_eq!(registered.index, 0);
    assert_eq!(registered.registry_version, 2);
    assert_eq!(registered.security_deposit, p.args.minimum_node_deposit);
    assert_eq!(registered.node, pda::node(&owner.pubkey()));
}

#[test]
fn register_node_requires_the_protocol_authority() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-auth", 1).remove(0);
    let owner = funded_owner(&mut p);
    let impostor = p.env.keypair();

    let args = p.register_node_args(&owner.pubkey(), &key, salt(1), 8_001);
    let ix = p.register_node_ix(&owner.pubkey(), &args, &impostor.pubkey());
    let failed = p.env.send_err(&[ix], &[&impostor, &owner]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
    assert_eq!(p.current_version(), 0);
}

#[test]
fn register_node_rejects_an_unroutable_endpoint() {
    let mut p = Protocol::start();
    let keys = crypto::node_keys("register-endpoint", 2);
    let owner = funded_owner(&mut p);

    let mut zero_ip = p.register_node_args(&owner.pubkey(), &keys[0], salt(1), 8_001);
    zero_ip.ip = [0, 0, 0, 0];
    let failed = register(&mut p, &owner, &zero_ip).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidNodeEndpoint);

    let mut zero_port = p.register_node_args(&owner.pubkey(), &keys[1], salt(2), 0);
    zero_port.port = 0;
    let failed = register(&mut p, &owner, &zero_port).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidNodeEndpoint);

    assert_eq!(p.current_version(), 0);
}

#[test]
fn register_node_rejects_a_zero_registration_salt() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-salt", 1).remove(0);
    let owner = funded_owner(&mut p);

    let args = p.register_node_args(&owner.pubkey(), &key, [0u8; 32], 8_001);
    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidRegistrationSalt);
}

#[test]
fn register_node_rejects_a_public_key_that_is_not_a_curve_point() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-curve", 1).remove(0);
    let owner = funded_owner(&mut p);

    let mut args = p.register_node_args(&owner.pubkey(), &key, salt(3), 8_001);
    // Valid prefix, garbage x coordinate.
    args.secp256k1_pubkey_compressed = [0xff; 33];
    args.secp256k1_pubkey_compressed[0] = 0x02;
    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::PubkeyNotOnCurve);
}

#[test]
fn register_node_rejects_a_proof_of_possession_for_a_different_node_id() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-pop-binding", 1).remove(0);
    let owner = funded_owner(&mut p);

    // A PoP that is valid — but signed over a node id built from a different salt.
    let other_node_id = crypto::derive_node_id(&owner.pubkey(), &key.compressed, &salt(200));
    let (pop_sig_r, pop_sig_s) = crypto::sign_pop(&molpha::ID, &other_node_id, &key, &[7u8; 32]);
    let mut args = p.register_node_args(&owner.pubkey(), &key, salt(3), 8_001);
    args.pop_sig_r = pop_sig_r;
    args.pop_sig_s = pop_sig_s;

    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidPoP);
}

#[test]
fn register_node_rejects_a_rogue_key_whose_secret_the_registrant_does_not_hold() {
    let mut p = Protocol::start();
    let keys = crypto::node_keys("register-rogue", 2);
    let owner = funded_owner(&mut p);

    // Claim `keys[0]`'s public key while signing with `keys[1]`'s secret.
    let node_id = crypto::derive_node_id(&owner.pubkey(), &keys[0].compressed, &salt(4));
    let (pop_sig_r, pop_sig_s) = crypto::sign_pop_with(
        &molpha::ID,
        &node_id,
        &keys[0].compressed,
        &keys[1].secret,
        &[11u8; 32],
    );
    let mut args = p.register_node_args(&owner.pubkey(), &keys[0], salt(4), 8_001);
    args.pop_sig_r = pop_sig_r;
    args.pop_sig_s = pop_sig_s;

    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidPoP);
}

#[test]
fn a_secp256k1_key_can_only_ever_back_one_node() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-key-claim", 1).remove(0);

    let first = funded_owner(&mut p);
    let args = p.register_node_args(&first.pubkey(), &key, salt(5), 8_001);
    register(&mut p, &first, &args).expect("first registration");

    // A different owner, a different salt — same key. The claim PDA blocks it.
    let second = funded_owner(&mut p);
    let args = p.register_node_args(&second.pubkey(), &key, salt(6), 8_002);
    let failed = register(&mut p, &second, &args).unwrap_err();
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
    assert_eq!(p.current_version(), 1);
}

#[test]
fn an_owner_can_only_back_one_node() {
    let mut p = Protocol::start();
    let keys = crypto::node_keys("register-owner-once", 2);
    let owner = funded_owner(&mut p);

    let args = p.register_node_args(&owner.pubkey(), &keys[0], salt(7), 8_001);
    register(&mut p, &owner, &args).expect("first registration");

    let args = p.register_node_args(&owner.pubkey(), &keys[1], salt(8), 8_002);
    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn register_node_rejects_a_registry_version_that_is_not_the_next_one() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-version", 1).remove(0);
    let owner = funded_owner(&mut p);

    let args = p.register_node_args(&owner.pubkey(), &key, salt(10), 8_001);
    let ix = p.register_node_ix_with_registry(
        &owner.pubkey(),
        &args,
        &p.authority.pubkey(),
        2, // current is 0, so 1 is the only legal successor
    );
    let failed = register_ix(&mut p, &owner, ix).unwrap_err();
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

#[test]
fn register_node_rejects_an_owner_who_cannot_cover_the_deposit() {
    let mut p = Protocol::start();
    let key = crypto::node_keys("register-poor", 1).remove(0);
    let owner = p.env.keypair();
    p.fund_usdc(&owner.pubkey(), p.args.minimum_node_deposit - 1);

    let args = p.register_node_args(&owner.pubkey(), &key, salt(11), 8_001);
    let failed = register(&mut p, &owner, &args).unwrap_err();
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("insufficient funds")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
    assert_eq!(p.current_version(), 0);
    assert!(!p.env.account_exists(&pda::node(&owner.pubkey())));
}

#[test]
fn registrations_append_in_order_and_keep_every_snapshot() {
    let mut p = Protocol::start();
    p.register_nodes("append-order", 4);

    assert_eq!(p.current_version(), 4);
    for version in 0..=4u32 {
        let snapshot = p.env.registry(version);
        assert_eq!(snapshot.version, version);
        assert_eq!(snapshot.node_count as u32, version);
        assert_eq!(snapshot.nodes, p.node_pdas()[..version as usize].to_vec());
    }
}

// ---------------------------------------------------------------------------
// deactivate_node
// ---------------------------------------------------------------------------

#[test]
fn deactivate_node_by_its_owner_removes_it_and_starts_the_withdrawal_cooldown() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-owner", 3);
    let target = p.nodes[1].pda;
    let owner = p.nodes[1].owner.insecure_clone();

    let ix = p.deactivate_node_ix(1, &owner.pubkey());
    let meta = p.env.send_ok(&[ix], &[&owner]);

    let node: Node = p.env.get(&target);
    let slot = p.env.slot();
    assert_eq!(node.status, NodeStatus::Deactivated);
    assert_eq!(
        node.withdrawable_slot,
        slot.saturating_add(p.args.withdrawal_cooldown_slots)
    );
    // Deactivation does not refund; the deposit stays locked until withdrawal.
    assert_eq!(node.locked_amount, p.args.minimum_node_deposit);

    // swap_remove: the last entry fills the hole.
    let after = p.env.registry(4);
    assert_eq!(after.node_count, 2);
    assert_eq!(after.nodes, vec![p.nodes[0].pda, p.nodes[2].pda]);
    assert!(!after.nodes.contains(&target));

    // The snapshot the node was in is still intact.
    let before = p.env.registry(3);
    assert_eq!(before.node_count, 3);
    assert!(before.nodes.contains(&target));

    let event: NodeDeactivated = only_event(&meta);
    assert_eq!(event.node, target);
    assert_eq!(event.index, 1);
    assert_eq!(event.new_registry_version, 5);
}

#[test]
fn deactivate_node_can_also_be_driven_by_the_protocol_authority() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-authority", 2);
    let authority = p.authority.insecure_clone();

    let ix = p.deactivate_node_ix(0, &authority.pubkey());
    p.env.send_ok(&[ix], &[&authority]);

    assert_eq!(
        p.env.get::<Node>(&p.nodes[0].pda).status,
        NodeStatus::Deactivated
    );
}

#[test]
fn deactivate_node_rejects_an_unrelated_caller() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-stranger", 2);
    let stranger = p.env.keypair();

    let ix = p.deactivate_node_ix(0, &stranger.pubkey());
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);
    assert_eq!(
        p.env.get::<Node>(&p.nodes[0].pda).status,
        NodeStatus::Active
    );
}

#[test]
fn deactivate_node_is_not_repeatable() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-twice", 2);
    let owner = p.nodes[0].owner.insecure_clone();

    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);

    // The node is gone from the registry, so build the second attempt by hand.
    let ix = p.deactivate_node_raw_ix(
        &owner.pubkey(),
        DeactivateNodeArgs {
            node_owner: owner.pubkey(),
            remove_index: 0,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NodeNotActive);
}

#[test]
fn deactivate_node_rejects_a_remove_index_that_points_at_another_node() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-index", 3);
    let owner = p.nodes[0].owner.insecure_clone();
    let current_version = p.current_version();

    let ix = p.deactivate_node_raw_ix(
        &owner.pubkey(),
        DeactivateNodeArgs {
            node_owner: owner.pubkey(),
            remove_index: 2, // node 0 lives at index 0
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);
    assert_eq!(p.current_version(), current_version);
}

#[test]
fn deactivate_node_rejects_a_remove_index_past_the_end_of_the_registry() {
    let mut p = Protocol::start();
    p.register_nodes("deactivate-oob", 2);
    let owner = p.nodes[0].owner.insecure_clone();

    let ix = p.deactivate_node_raw_ix(
        &owner.pubkey(),
        DeactivateNodeArgs {
            node_owner: owner.pubkey(),
            remove_index: 99,
        },
    );
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeIndex);
}

// ---------------------------------------------------------------------------
// update_node_endpoint
// ---------------------------------------------------------------------------

#[test]
fn update_node_endpoint_rewrites_ip_and_port_for_the_operator() {
    let mut p = Protocol::start();
    p.register_nodes("endpoint", 1);
    let owner = p.nodes[0].owner.insecure_clone();

    let ix = p.update_node_endpoint_ix(&owner.pubkey(), [203, 0, 113, 5], 4_242);
    let meta = p.env.send_ok(&[ix], &[&owner]);

    let node: Node = p.env.get(&p.nodes[0].pda);
    assert_eq!(node.ip, [203, 0, 113, 5]);
    assert_eq!(node.port, 4_242);

    let event: NodeEndpointUpdated = only_event(&meta);
    assert_eq!(event.node, p.nodes[0].pda);
    assert_eq!(event.port, 4_242);
}

#[test]
fn update_node_endpoint_rejects_an_unroutable_endpoint() {
    let mut p = Protocol::start();
    p.register_nodes("endpoint-invalid", 1);
    let owner = p.nodes[0].owner.insecure_clone();

    let ix = p.update_node_endpoint_ix(&owner.pubkey(), [0, 0, 0, 0], 4_242);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeEndpoint);

    let ix = p.update_node_endpoint_ix(&owner.pubkey(), [203, 0, 113, 5], 0);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeEndpoint);
}

#[test]
fn update_node_endpoint_rejects_anyone_but_the_node_owner() {
    let mut p = Protocol::start();
    p.register_nodes("endpoint-stranger", 1);
    let stranger = p.env.keypair();

    // The node PDA is owner-seeded, so a stranger's call names an account that does not exist.
    let ix = p.update_node_endpoint_ix(&stranger.pubkey(), [203, 0, 113, 5], 4_242);
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_anchor_error(
        &failed,
        anchor_lang::error::ErrorCode::AccountNotInitialized,
    );
}

#[test]
fn update_node_endpoint_rejects_a_deactivated_node() {
    let mut p = Protocol::start();
    p.register_nodes("endpoint-deactivated", 2);
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);

    let ix = p.update_node_endpoint_ix(&owner.pubkey(), [203, 0, 113, 5], 4_242);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NodeNotActive);
}

// ---------------------------------------------------------------------------
// update_registry_policy
// ---------------------------------------------------------------------------

#[test]
fn update_registry_policy_carries_membership_into_a_new_snapshot() {
    let mut p = Protocol::start();
    p.register_nodes("policy", 3);
    let before = p.env.registry(3);
    let authority = p.authority.insecure_clone();

    let ix = p.update_registry_policy_ix(&authority.pubkey(), 4, 5);
    let meta = p.env.send_ok(&[ix], &[&authority]);

    let after = p.env.registry(4);
    assert_eq!(after.version, 4);
    assert_eq!(after.node_count, before.node_count);
    assert_eq!(after.nodes, before.nodes);
    assert_eq!(after.redundancy_buffer, 5);
    assert_eq!(
        p.env.registry(3).redundancy_buffer,
        p.args.redundancy_buffer,
        "the previous snapshot keeps its own policy"
    );
    assert_eq!(
        p.env
            .get::<RegistryState>(&pda::registry_state())
            .current_version,
        4
    );

    let event: RegistryUpdated = only_event(&meta);
    assert_eq!(event.new_version, 5);
    assert_eq!(event.node_count, before.node_count);
    assert_eq!(event.redundancy_buffer, 5);
    assert!(matches!(event.reason, RegistryVersionReason::PolicyChanged));
}

#[test]
fn update_registry_policy_rejects_a_no_op_change() {
    let mut p = Protocol::start();
    p.register_nodes("policy-noop", 1);
    let authority = p.authority.insecure_clone();
    let buffer = p.args.redundancy_buffer;

    let ix = p.update_registry_policy_ix(&authority.pubkey(), 2, buffer);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::RegistryPolicyUnchanged);
    assert_eq!(p.current_version(), 1);
}

#[test]
fn update_registry_policy_rejects_a_version_that_is_not_the_next_one() {
    let mut p = Protocol::start();
    p.register_nodes("policy-version", 1);
    let authority = p.authority.insecure_clone();

    let ix = p.update_registry_policy_ix(&authority.pubkey(), 5, 9);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}

#[test]
fn update_registry_policy_rejects_a_caller_that_is_not_the_authority() {
    let mut p = Protocol::start();
    p.register_nodes("policy-auth", 1);
    let impostor = p.env.keypair();

    let ix = p.update_registry_policy_ix(&impostor.pubkey(), 2, 9);
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
}

// ---------------------------------------------------------------------------
// snapshot immutability
// ---------------------------------------------------------------------------

#[test]
fn a_historical_snapshot_still_verifies_after_membership_and_policy_change() {
    let mut p = Protocol::start();
    p.register_nodes("historical", 6);

    // An attestation signed against version 6.
    let historical = p.sign_duty(Duty::new(6, 3));

    // Membership churn plus a policy change on top of it.
    let owner = p.nodes[5].owner.insecure_clone();
    let ix = p.deactivate_node_ix(5, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
    let authority = p.authority.insecure_clone();
    let next = p.current_version() + 1;
    let ix = p.update_registry_policy_ix(&authority.pubkey(), next, 4);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(p.current_version(), 8);

    // The version-6 attestation still verifies against the version-6 snapshot.
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &historical);
    p.env.send_ok(&[ix], &[&submitter]);

    // And the historical snapshot is byte-for-byte what it was.
    let v6 = p.env.registry(6);
    assert_eq!(v6.node_count, 6);
    assert_eq!(v6.redundancy_buffer, p.args.redundancy_buffer);
}

/// Registering the 4-node fixture and reading its keys back proves `register_nodes` keeps the
/// registry order and the key material in lockstep — the assumption every signing test rests on.
#[test]
fn registered_node_order_matches_the_key_material() {
    let mut p = Protocol::start();
    p.register_nodes("order-check", 4);
    let registry = p.env.registry(4);

    for (bit, node_pda) in registry.nodes.iter().enumerate() {
        let node: Node = p.env.get(node_pda);
        let key: &NodeKey = &p.nodes[bit].key;
        assert_eq!(node.secp256k1_pubkey_x, key.x);
        assert_eq!(node.secp256k1_pubkey_y, key.y);
    }
}
