//! Reusable node-punishment mechanics — applying a [`Penalty`] to a node's funds/status,
//! evicting victims from a fresh registry snapshot, and executing the shared sentence tail via
//! [`execute_punishment`]. Offence-specific instructions prove their own guilty set, then call
//! [`execute_punishment`] for slash / eviction / bounty / case init / registry-version event.
//! Round-dispute bounty (`dispute_round`) stays inline — different account wiring.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount, Transfer};
use molpha_verifier::bitmap::bitmap_bit_set;

use crate::{
    error::MolphaError,
    events::{NodePunished, RegistryVersionCreated, RegistryVersionReason},
    state::{
        MembershipAction, Node, NodeStatus, OffenceKind, Penalty, ProtocolConfig, PunishmentCase,
        Registry, RegistryState,
    },
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
    (amount as u128 * bps as u128 / 10_000) as u64
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

/// Outcome of a [`punish_guilty_nodes`] batch.
#[derive(Clone, Debug, Default)]
pub struct PunishmentBatch {
    /// Nodes actually slashed, in resolution order. Feeds [`evict_nodes`].
    /// Length may be less than `guilty_bitmap.count_ones()` when a guilty bit belongs to a node
    /// already `Tombstoned` by an earlier case; that bit is skipped rather than erroring.
    pub victims: Vec<Pubkey>,
    pub slashed_total: u64,
}

/// Apply `offence`'s [`Penalty`] to every guilty node and emit a [`NodePunished`] event per victim.
///
/// `resolved` and `remaining_accounts` must be the same length and pairwise-aligned — the shape
/// [`crate::state::resolve_registry_signers_indexed`] and `ctx.remaining_accounts` already share.
/// A bit in `guilty_bitmap` outside `resolved`'s bit positions is simply never matched, so callers
/// must resolve at least the guilty bitmap (a superset, e.g. a signature's full signer set, is
/// fine).
///
/// Already-`Tombstoned` nodes are skipped rather than propagating [`apply_penalty`]'s
/// `NodeTombstoned` error: one previously-punished node named in a batch must not shield every
/// other guilty node in the same transaction from punishment.
pub fn punish_guilty_nodes<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    resolved: &[(usize, molpha_verifier::SignerXy)],
    guilty_bitmap: &[u8; 32],
    offence: OffenceKind,
    now: i64,
    program_id: &Pubkey,
) -> Result<PunishmentBatch> {
    let penalty = offence.penalty();
    let mut batch = PunishmentBatch::default();

    for ((bit_pos, _), account_info) in resolved.iter().zip(remaining_accounts.iter()) {
        if !bitmap_bit_set(guilty_bitmap, *bit_pos) {
            continue;
        }
        require!(
            account_info.owner == program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(account_info)?;
        if node.status == NodeStatus::Tombstoned {
            continue;
        }
        let outcome = apply_penalty(&mut node, penalty, now)?;
        batch.slashed_total = batch.slashed_total.saturating_add(outcome.total());

        emit!(NodePunished {
            node: account_info.key(),
            owner: node.owner,
            offence,
            slashed_locked: outcome.slashed_locked,
            slashed_rewards: outcome.slashed_rewards,
            new_status: node.status,
            punished_at: now,
        });
        batch.victims.push(account_info.key());
        node.exit(program_id)?;
    }

    Ok(batch)
}

/// Evict `victims` from a fresh registry snapshot copied from `current`.
///
/// Victims are located by scanning `nodes[..node_count]` on every iteration rather than by a
/// client-supplied index: `Registry::swap_remove` moves the last node into the removed slot, so
/// removing more than one node in a batch by stale indices is a footgun once an earlier removal
/// has shifted a later victim. Silently skips a victim no longer present in the registry (e.g. it
/// already self-deactivated) — punishment always applies the fund/status penalty via
/// [`apply_penalty`] regardless of current registry membership.
pub fn evict_nodes(
    current: &Registry,
    next: &mut Registry,
    version: u32,
    bump: u8,
    victims: &[Pubkey],
) -> Result<()> {
    next.copy_from_previous(current, version, bump);
    for victim in victims {
        let found = (0..usize::from(next.node_count))
            .find(|&i| Pubkey::new_from_array(next.nodes[i]) == *victim);
        if let Some(index) = found {
            next.swap_remove(index as u16, *victim)?;
        }
    }
    Ok(())
}

/// Mutable accounts shared by every node-punishment sentence.
pub struct ExecutePunishmentAccounts<'a, 'info> {
    pub registry_state: &'a mut Account<'info, RegistryState>,
    pub current_registry: &'a AccountLoader<'info, Registry>,
    pub new_registry: &'a AccountLoader<'info, Registry>,
    pub new_registry_bump: u8,
    pub protocol_config: &'a Account<'info, ProtocolConfig>,
    pub punishment_case: &'a mut Account<'info, PunishmentCase>,
    pub punishment_case_bump: u8,
    pub reporter: Pubkey,
    pub treasury: &'a Account<'info, TokenAccount>,
    pub reporter_usdc: &'a Account<'info, TokenAccount>,
    pub token_program: Pubkey,
}

/// Proof output from an offence-specific handler, fed into [`execute_punishment`].
pub struct ExecutePunishmentInput<'a, 'info> {
    pub remaining_accounts: &'info [AccountInfo<'info>],
    pub program_id: &'info Pubkey,
    pub resolved: &'a [(usize, molpha_verifier::SignerXy)],
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub offence: OffenceKind,
    pub registry_version: u32,
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

/// Values the offence-specific `*Proven` event needs after the shared sentence runs.
#[derive(Clone, Copy, Debug)]
pub struct ExecutePunishmentOutcome {
    pub slashed_total: u64,
    pub bounty_paid: u64,
    pub previous_version: u32,
    pub node_count_after: u16,
    pub redundancy_buffer_after: u8,
    pub now: i64,
    pub now_slot: u64,
}

/// Shared sentence: slash guilty nodes, evict them into a successor registry, pay the reporter
/// bounty, init `PunishmentCase`, and emit `RegistryVersionCreated`.
pub fn execute_punishment<'a, 'b, 'info>(
    accounts: ExecutePunishmentAccounts<'a, 'info>,
    input: ExecutePunishmentInput<'b, 'info>,
) -> Result<ExecutePunishmentOutcome> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let now_slot = clock.slot;

    let batch = punish_guilty_nodes(
        input.remaining_accounts,
        input.resolved,
        &input.guilty_bitmap,
        input.offence,
        now,
        input.program_id,
    )?;
    require!(!batch.victims.is_empty(), MolphaError::NothingToPunish);
    let slashed_total = batch.slashed_total;

    let previous_version = accounts.registry_state.current_version;
    let (node_count_after, redundancy_buffer_after) = {
        let current = accounts.current_registry.load()?;
        let mut next = accounts.new_registry.load_init()?;
        evict_nodes(
            &current,
            &mut next,
            input.new_registry_version,
            accounts.new_registry_bump,
            &batch.victims,
        )?;
        (next.node_count, next.redundancy_buffer)
    };
    accounts.registry_state.current_version = input.new_registry_version;

    let bounty = bps_of(slashed_total, accounts.protocol_config.challenger_bounty_bps);
    if bounty > 0 {
        let bump = [accounts.protocol_config.bump];
        let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
        token::transfer(
            CpiContext::new_with_signer(
                accounts.token_program,
                Transfer {
                    from: accounts.treasury.to_account_info(),
                    to: accounts.reporter_usdc.to_account_info(),
                    authority: accounts.protocol_config.to_account_info(),
                },
                &[signer_seeds],
            ),
            bounty,
        )?;
    }

    accounts.punishment_case.init(
        input.offence,
        accounts.reporter,
        input.registry_version,
        input.guilty_bitmap,
        input.guilty_count,
        slashed_total,
        bounty,
        input.evidence_hash,
        now_slot,
        accounts.punishment_case_bump,
    );

    emit!(RegistryVersionCreated {
        previous_version,
        new_version: input.new_registry_version,
        node_count: node_count_after,
        redundancy_buffer: redundancy_buffer_after,
        reason: RegistryVersionReason::NodePunished,
        created_at: now,
    });

    Ok(ExecutePunishmentOutcome {
        slashed_total,
        bounty_paid: bounty,
        previous_version,
        node_count_after,
        redundancy_buffer_after,
        now,
        now_slot,
    })
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
    fn half_slash_tombstones_and_leaves_the_remainder() {
        let mut node = node_fixture(100_000_001, 3, NodeStatus::Active);
        let penalty = Penalty {
            slash_bps: 5_000, // 50% — OffenceKind::NonceReuse
            membership: MembershipAction::Tombstone,
            freeze_secs: 0,
        };
        let outcome = apply_penalty(&mut node, penalty, 100).unwrap();
        assert_eq!(outcome.slashed_locked, 50_000_000); // floor(100_000_001 * 0.5)
        assert_eq!(outcome.slashed_rewards, 1); // floor(3 * 0.5)
        assert_eq!(node.locked_amount, 50_000_001);
        assert_eq!(node.claimable_rewards, 2);
        assert_eq!(node.status, NodeStatus::Tombstoned);
        // The `Tombstone` arm must not write `frozen_until` — that field is `Freeze`-only.
        assert_eq!(node.frozen_until, 0);
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

        evict_nodes(&current, &mut next, 1, 0, &[n0, n3]).unwrap();
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

        evict_nodes(&current, &mut next, 1, 0, &[ghost]).unwrap();
        assert_eq!(next.node_count, 1);
    }
}
