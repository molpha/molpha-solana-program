//! Registry snapshot mechanics: strict version succession, `copy_from_previous` /
//! `swap_remove_node(s)` isolation, grace expiry via `is_active`, and the rescan that
//! survives the index shift `swap_remove` causes.
//!
//! Pure unit tests — no VM.

use anchor_lang::prelude::Pubkey;
use molpha::state::{Registry, MAX_REGISTRY_NODES};

fn registry_with(nodes: &[Pubkey]) -> Registry {
    let mut nodes_arr = [[0u8; 32]; MAX_REGISTRY_NODES];
    for (i, n) in nodes.iter().enumerate() {
        nodes_arr[i] = n.to_bytes();
    }
    Registry {
        version: 0,
        node_count: nodes.len() as u16,
        redundancy_buffer: 0,
        bump: 0,
        nodes: nodes_arr,
        grace_active_until: 0,
        active_from: 0,
    }
}

#[test]
fn snapshot_copy_and_swap_remove_do_not_mutate_source() {
    let first = Pubkey::new_unique();
    let second = Pubkey::new_unique();
    let third = Pubkey::new_unique();
    let mut source = Registry {
        version: 7,
        node_count: 3,
        redundancy_buffer: 2,
        bump: 1,
        nodes: [[0; 32]; MAX_REGISTRY_NODES],
        grace_active_until: 0,
        active_from: 0,
    };
    source.nodes[0] = first.to_bytes();
    source.nodes[1] = second.to_bytes();
    source.nodes[2] = third.to_bytes();

    let mut next = Registry {
        version: 0,
        node_count: 0,
        redundancy_buffer: 0,
        bump: 0,
        nodes: [[0; 32]; MAX_REGISTRY_NODES],
        grace_active_until: 0,
        active_from: 0,
    };
    next.copy_from_previous(&mut source, 8, 3, 2);
    next.swap_remove_node(second).unwrap();

    assert_eq!(source.node_count, 3);
    assert_eq!(source.node(1).unwrap(), second);
    assert_eq!(source.grace_active_until, 8);
    assert_eq!(next.version, 8);
    assert_eq!(next.active_from, 3);
    assert_eq!(next.node_count, 2);
    assert_eq!(next.node(1).unwrap(), third);
    assert_eq!(next.nodes[2], [0; 32]);
}

#[test]
fn swap_remove_batch_handles_swap_index_hazard() {
    let n0 = Pubkey::new_unique();
    let n1 = Pubkey::new_unique();
    let n2 = Pubkey::new_unique();
    let n3 = Pubkey::new_unique();
    let mut current = registry_with(&[n0, n1, n2, n3]);
    let mut next = registry_with(&[]);

    next.copy_from_previous(&mut current, 1, 1, 0);
    next.swap_remove_nodes(&[n0, n3]).unwrap();
    assert_eq!(next.node_count, 2);
    let remaining: Vec<Pubkey> = (0..usize::from(next.node_count))
        .map(|i| Pubkey::new_from_array(next.nodes[i]))
        .collect();
    assert!(remaining.contains(&n1));
    assert!(remaining.contains(&n2));
    assert_eq!(current.node_count, 4);
}

#[test]
fn swap_remove_batch_skips_absent_victim() {
    let n0 = Pubkey::new_unique();
    let ghost = Pubkey::new_unique();
    let mut current = registry_with(&[n0]);
    let mut next = registry_with(&[]);

    next.copy_from_previous(&mut current, 1, 1, 0);
    next.swap_remove_nodes(&[ghost]).unwrap();
    assert_eq!(next.node_count, 1);
}

#[test]
fn registry_is_active_until_grace_expires() {
    let registry = Registry {
        grace_active_until: 0,
        ..registry_with(&[])
    };
    assert!(registry.is_active(0));
    assert!(registry.is_active(i64::MAX));

    let mut expired = registry_with(&[]);
    expired.grace_active_until = 100;
    assert!(expired.is_active(99));
    assert!(!expired.is_active(100));
    assert!(!expired.is_active(101));

    expired.active_from = 20;
    assert!(!expired.is_active(19));
    assert!(expired.is_active(20));
}
