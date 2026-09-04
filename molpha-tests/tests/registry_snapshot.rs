//! Registry snapshot mechanics: strict version succession, `copy_from_previous` / `swap_remove`
//! isolation, and the `evict_nodes` rescan that survives the index shift `swap_remove` causes.
//!
//! Pure unit tests — no VM.

use anchor_lang::prelude::Pubkey;
use molpha::state::{Registry, RegistryState, MAX_REGISTRY_NODES};

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
    }
}

#[test]
fn advance_version_moves_current_to_next_and_increments_next() {
    let mut state = RegistryState {
        current_version: 41,
        next_version: 42,
        bump: 1,
    };
    state.advance_version();
    assert_eq!(state.current_version, 42);
    assert_eq!(state.next_version, 43);
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
    };
    next.swap_remove(&source, second, 8).unwrap();

    assert_eq!(source.node_count, 3);
    assert_eq!(source.node(1).unwrap(), second);
    assert_eq!(next.version, 8);
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
    let current = registry_with(&[n0, n1, n2, n3]);
    let mut next = registry_with(&[]);

    next.swap_remove_batch(&current, &[n0, n3], 1).unwrap();
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
    let current = registry_with(&[n0]);
    let mut next = registry_with(&[]);

    next.swap_remove_batch(&current, &[ghost], 1).unwrap();
    assert_eq!(next.node_count, 1);
}
