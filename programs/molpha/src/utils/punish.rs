//! Reusable node-punishment mechanics — applying a [`Penalty`] to a node's funds/status, and
//! evicting a batch of victims from a fresh registry snapshot. Offence-specific instructions
//! (e.g. `punish_equivocation`) prove their own guilty set and call these directly; the CPI that
//! pays the reporter bounty stays in the instruction handler, matching how `dispute_round` does
//! its bounty transfer inline rather than through a shared helper.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    state::{MembershipAction, Node, NodeStatus, Penalty, Registry},
};

/// Slashed amounts from a single [`apply_penalty`] call.
#[derive(Clone, Copy, Debug, Default)]
pub struct PenaltyOutcome {
    pub slashed_locked: u64,
    pub slashed_rewards: u64,
}

impl PenaltyOutcome {
    pub fn total(&self) -> u64 {
        self.slashed_locked.saturating_add(self.slashed_rewards)
    }
}

fn bps_of(amount: u64, bps: u16) -> u64 {
    ((amount as u128) * (bps as u128) / 10_000) as u64
}

/// Apply `penalty` to `node`'s fund and status fields.
///
/// Fund effects always run; registry-membership eviction is resolved separately by the caller via
/// [`evict_nodes`], since that needs the current registry snapshot which this function does not
/// have.
///
/// Rejects an already-`Tombstoned` node: the terminal state is idempotent, and silently no-op'ing
/// a re-slash of a zeroed balance would mask a double-submission bug in the caller instead of
/// surfacing it.
pub fn apply_penalty(node: &mut Node, penalty: Penalty, now: i64) -> Result<PenaltyOutcome> {
    require!(
        node.status != NodeStatus::Tombstoned,
        MolphaError::NodeTombstoned
    );

    let slashed_locked = bps_of(node.locked_amount, penalty.slash_bps);
    let slashed_rewards = bps_of(node.claimable_rewards, penalty.slash_bps);
    node.locked_amount = node.locked_amount.saturating_sub(slashed_locked);
    node.claimable_rewards = node.claimable_rewards.saturating_sub(slashed_rewards);
    node.punished_at = now;

    match penalty.membership {
        MembershipAction::None => {}
        MembershipAction::Freeze => {
            node.status = NodeStatus::Frozen;
            node.frozen_until = now.saturating_add(penalty.freeze_secs);
        }
        MembershipAction::Tombstone => {
            node.status = NodeStatus::Tombstoned;
        }
    }

    Ok(PenaltyOutcome {
        slashed_locked,
        slashed_rewards,
    })
}

/// Evict `victims` from a fresh registry snapshot copied from `current`.
///
/// Victims are located by scanning `nodes[..node_count]` on every iteration rather than by a
/// client-supplied index: `Registry::swap_remove` moves the last node into the removed slot, so
/// removing more than one node in a batch by stale indices is a footgun once an earlier removal
/// has shifted a later victim. Silently skips a victim no longer present in the registry (e.g. it
/// already self-deactivated) — punishment always applies the fund/status penalty via
/// [`apply_penalty`] regardless of current registry membership.
///
/// Returns the victims actually removed, in the order they were removed.
pub fn evict_nodes(
    current: &Registry,
    next: &mut Registry,
    version: u32,
    bump: u8,
    victims: &[Pubkey],
) -> Result<Vec<Pubkey>> {
    next.copy_from_previous(current, version, bump);
    let mut removed = Vec::with_capacity(victims.len());
    for victim in victims {
        let found = (0..usize::from(next.node_count))
            .find(|&i| Pubkey::new_from_array(next.nodes[i]) == *victim);
        if let Some(index) = found {
            next.swap_remove(index as u16, *victim)?;
            removed.push(*victim);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MAX_REGISTRY_NODES;

    fn node_fixture(locked: u64, rewards: u64, status: NodeStatus) -> Node {
        Node {
            owner: Pubkey::new_unique(),
            secp256k1_pubkey_x: [0u8; 32],
            secp256k1_pubkey_y: [0u8; 32],
            status,
            ip: [0, 0, 0, 0],
            port: 0,
            locked_amount: locked,
            claimable_rewards: rewards,
            registered_at: 0,
            deactivated_at: 0,
            withdrawable_at: 0,
            frozen_until: 0,
            punished_at: 0,
            bump: 0,
        }
    }

    #[test]
    fn full_slash_zeroes_balances_and_tombstones() {
        let mut node = node_fixture(1_000, 42, NodeStatus::Active);
        let penalty = Penalty {
            slash_bps: 10_000,
            membership: MembershipAction::Tombstone,
            freeze_secs: 0,
        };
        let outcome = apply_penalty(&mut node, penalty, 100).unwrap();
        assert_eq!(outcome.slashed_locked, 1_000);
        assert_eq!(outcome.slashed_rewards, 42);
        assert_eq!(outcome.total(), 1_042);
        assert_eq!(node.locked_amount, 0);
        assert_eq!(node.claimable_rewards, 0);
        assert_eq!(node.status, NodeStatus::Tombstoned);
        assert_eq!(node.punished_at, 100);
    }

    #[test]
    fn partial_slash_rounds_down_and_freezes() {
        let mut node = node_fixture(999, 0, NodeStatus::Active);
        let penalty = Penalty {
            slash_bps: 2_500, // 25%
            membership: MembershipAction::Freeze,
            freeze_secs: 3_600,
        };
        let outcome = apply_penalty(&mut node, penalty, 500).unwrap();
        assert_eq!(outcome.slashed_locked, 249); // floor(999 * 0.25)
        assert_eq!(node.locked_amount, 750);
        assert_eq!(node.status, NodeStatus::Frozen);
        assert_eq!(node.frozen_until, 4_100);
    }

    #[test]
    fn zero_bps_is_a_pure_status_transition() {
        let mut node = node_fixture(1_000, 1_000, NodeStatus::Active);
        let penalty = Penalty {
            slash_bps: 0,
            membership: MembershipAction::None,
            freeze_secs: 0,
        };
        let outcome = apply_penalty(&mut node, penalty, 1).unwrap();
        assert_eq!(outcome.slashed_locked, 0);
        assert_eq!(outcome.slashed_rewards, 0);
        assert_eq!(node.locked_amount, 1_000);
        assert_eq!(node.status, NodeStatus::Active);
    }

    #[test]
    fn tombstoned_node_cannot_be_punished_again() {
        let mut node = node_fixture(0, 0, NodeStatus::Tombstoned);
        let penalty = Penalty {
            slash_bps: 10_000,
            membership: MembershipAction::Tombstone,
            freeze_secs: 0,
        };
        assert!(apply_penalty(&mut node, penalty, 1).is_err());
    }

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
    fn evict_nodes_handles_swap_index_hazard() {
        // Removing node[0] first swaps the last node (n3) into slot 0. A later victim that was
        // originally at index 3 must still be found by rescanning, not by a stale index.
        let n0 = Pubkey::new_unique();
        let n1 = Pubkey::new_unique();
        let n2 = Pubkey::new_unique();
        let n3 = Pubkey::new_unique();
        let current = registry_with(&[n0, n1, n2, n3]);
        let mut next = registry_with(&[]);

        let removed = evict_nodes(&current, &mut next, 1, 0, &[n0, n3]).unwrap();
        assert_eq!(removed, vec![n0, n3]);
        assert_eq!(next.node_count, 2);
        let remaining: Vec<Pubkey> = (0..usize::from(next.node_count))
            .map(|i| Pubkey::new_from_array(next.nodes[i]))
            .collect();
        assert!(remaining.contains(&n1));
        assert!(remaining.contains(&n2));
        // Source snapshot is untouched.
        assert_eq!(current.node_count, 4);
    }

    #[test]
    fn evict_nodes_skips_absent_victim() {
        let n0 = Pubkey::new_unique();
        let ghost = Pubkey::new_unique();
        let current = registry_with(&[n0]);
        let mut next = registry_with(&[]);

        let removed = evict_nodes(&current, &mut next, 1, 0, &[ghost]).unwrap();
        assert!(removed.is_empty());
        assert_eq!(next.node_count, 1);
    }
}
