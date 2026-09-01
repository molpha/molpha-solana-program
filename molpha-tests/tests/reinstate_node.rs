//! `reinstate_node`: bringing a frozen node back into the registry.
//!
//! Every penalty in the current table tombstones, so `NodeStatus::Frozen` is not reachable
//! through the instruction set today. These tests plant the state directly (see
//! `Protocol::freeze_node`) so the reinstatement path stays covered ahead of the first penalty
//! that uses `MembershipAction::Freeze`.

use molpha::error::MolphaError;
use molpha::events::NodeReinstated;
use molpha::state::{Node, NodeStatus};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

const NODE_COUNT: usize = 4;

fn frozen_protocol(freeze_for: i64) -> (Protocol, usize) {
    let mut p = Protocol::start();
    p.register_nodes("reinstate", NODE_COUNT);
    let frozen_until = p.env.unix_timestamp() + freeze_for;
    p.freeze_node(1, frozen_until);
    (p, 1)
}

#[test]
fn a_thawed_node_is_reinstated_by_anyone_and_rejoins_the_registry() {
    let (mut p, index) = frozen_protocol(600);
    let node_pda = p.nodes[index].pda;
    let owner = p.nodes[index].owner.pubkey();
    assert!(!p
        .env
        .registry(p.current_version())
        .nodes
        .contains(&node_pda));

    p.env.advance_seconds(601);
    let stranger = p.env.keypair();
    let expected_version = p.current_version() + 1;
    let ix = p.reinstate_node_ix(&stranger.pubkey(), &owner);
    let meta = p.env.send_ok(&[ix], &[&stranger]);

    assert_eq!(p.env.get::<Node>(&node_pda).status, NodeStatus::Active);
    let registry = p.env.registry(expected_version);
    assert_eq!(registry.node_count as usize, NODE_COUNT);
    assert!(registry.nodes.contains(&node_pda));
    assert_eq!(p.current_version(), expected_version);

    let event: NodeReinstated = only_event(&meta);
    assert_eq!(event.node, node_pda);
    assert_eq!(event.owner, owner);
    assert_eq!(event.registry_version, p.current_version() + 1);
}

#[test]
fn a_still_frozen_node_cannot_be_reinstated_by_a_stranger() {
    let (mut p, index) = frozen_protocol(600);
    let owner = p.nodes[index].owner.pubkey();
    let stranger = p.env.keypair();

    let ix = p.reinstate_node_ix(&stranger.pubkey(), &owner);
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_molpha_error(&failed, MolphaError::NodeStillFrozen);
    assert_eq!(
        p.env.get::<Node>(&p.nodes[index].pda).status,
        NodeStatus::Frozen
    );
}

#[test]
fn a_still_frozen_node_cannot_be_reinstated_early_by_its_owner() {
    let (mut p, index) = frozen_protocol(600);
    let owner = p.nodes[index].owner.insecure_clone();

    let ix = p.reinstate_node_ix(&owner.pubkey(), &owner.pubkey());
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert_molpha_error(&failed, MolphaError::NodeStillFrozen);
    assert_eq!(
        p.env.get::<Node>(&p.nodes[index].pda).status,
        NodeStatus::Frozen
    );
}

#[test]
fn the_protocol_authority_may_thaw_a_node_early() {
    let (mut p, index) = frozen_protocol(10_000);
    let owner = p.nodes[index].owner.pubkey();
    let authority = p.authority.insecure_clone();

    let ix = p.reinstate_node_ix(&authority.pubkey(), &owner);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(
        p.env.get::<Node>(&p.nodes[index].pda).status,
        NodeStatus::Active
    );
}

#[test]
fn an_active_node_has_nothing_to_reinstate() {
    let mut p = Protocol::start();
    p.register_nodes("reinstate-active", NODE_COUNT);
    let owner = p.nodes[0].owner.pubkey();
    let caller = p.env.keypair();

    let ix = p.reinstate_node_ix(&caller.pubkey(), &owner);
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::NodeNotFrozen);
}

#[test]
fn a_voluntarily_deactivated_node_is_not_reinstated_this_way() {
    let mut p = Protocol::start();
    p.register_nodes("reinstate-deactivated", NODE_COUNT);
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);

    let caller = p.env.keypair();
    let ix = p.reinstate_node_ix(&caller.pubkey(), &owner.pubkey());
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::NodeNotFrozen);
}

#[test]
fn a_tombstoned_node_is_never_reinstated_even_by_the_authority() {
    let mut p = Protocol::start();
    p.register_nodes("reinstate-tombstoned", NODE_COUNT);
    let owner = p.nodes[0].owner.insecure_clone();
    let ix = p.deactivate_node_ix(0, &owner.pubkey());
    p.env.send_ok(&[ix], &[&owner]);
    let node_pda = p.nodes[0].pda;
    p.overwrite_node(&node_pda, |node| {
        node.status = NodeStatus::Tombstoned;
    });

    let authority = p.authority.insecure_clone();
    let ix = p.reinstate_node_ix(&authority.pubkey(), &owner.pubkey());
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::NodeNotFrozen);
}

#[test]
fn a_frozen_node_that_is_still_in_the_registry_is_not_added_twice() {
    let mut p = Protocol::start();
    p.register_nodes("reinstate-duplicate", NODE_COUNT);
    // Freeze in place, without removing the node from the registry first.
    let node_pda = p.nodes[0].pda;
    p.overwrite_node(&node_pda, |node| {
        node.status = NodeStatus::Frozen;
        node.frozen_until = 0;
    });

    let owner = p.nodes[0].owner.pubkey();
    let caller = p.env.keypair();
    let ix = p.reinstate_node_ix(&caller.pubkey(), &owner);
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::NodeAlreadyInRegistry);
}

#[test]
fn reinstatement_requires_the_next_registry_version() {
    let (mut p, index) = frozen_protocol(0);
    let owner = p.nodes[index].owner.pubkey();
    let caller = p.env.keypair();

    let skipped = p.current_version() + 2;
    let ix = p.reinstate_node_raw_ix(&caller.pubkey(), &owner, skipped);

    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintSeeds);
}
