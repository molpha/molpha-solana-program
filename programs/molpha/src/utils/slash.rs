//! Shared node punishment: apply penalties, evict victims, and run the common sentence tail.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount, Transfer};
use molpha_verifier::bitmap::bitmap_bit_set;

use crate::{
    error::{map_attestation_err, MolphaError},
    events::{NodePunished, RegistryVersionCreated, RegistryVersionReason},
    state::{
        MembershipAction, Node, NodeStatus, OffenceKind, Penalty, ProtocolConfig, PunishmentCase,
        Registry, RegistryState, SignedStatement,
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

/// Apply `penalty` to `node`. Registry eviction is handled separately by the caller.
pub fn apply_penalty(node: &mut Node, penalty: Penalty, now: i64) -> Result<PenaltyOutcome> {
    require!(
        node.status != NodeStatus::Tombstoned,
        MolphaError::NodeTombstoned
    );

    let slashed_locked = bps_of(node.locked_amount, penalty.slash_bps);
    let slashed_rewards = bps_of(node.claimable_rewards, penalty.slash_bps);
    node.locked_amount = node.locked_amount.saturating_sub(slashed_locked);
    node.claimable_rewards = node.claimable_rewards.saturating_sub(slashed_rewards);

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
    /// Nodes slashed in resolution order; may be shorter than `guilty_bitmap` if some are tombstoned.
    pub victims: Vec<Pubkey>,
    pub slashed_total: u64,
}

/// Slash every guilty node. Already-tombstoned nodes are skipped so one stale bit cannot block the batch.
pub fn slash_guilty_nodes<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    resolved: &[(usize, molpha_verifier::SignerXy)],
    guilty_bitmap: &[u8; 32],
    offence: OffenceKind,
    now: i64,
    withdrawal_cooldown_secs: i64,
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
        if node.status == NodeStatus::Tombstoned {
            require!(
                withdrawal_cooldown_secs >= 0,
                MolphaError::InvalidWithdrawalCooldown
            );
            node.deactivated_at = now;
            node.withdrawable_at = now
                .checked_add(withdrawal_cooldown_secs)
                .ok_or(MolphaError::WithdrawalArithmeticOverflow)?;
        }
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

/// Evict `victims` into a fresh registry snapshot. Rescans by pubkey because `swap_remove` shifts indices.
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
pub struct SlashAccounts<'a, 'info> {
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
pub struct SlashInput<'a, 'info> {
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
pub struct SlashOutcome {
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
pub fn slash_nodes<'a, 'b, 'info>(
    accounts: SlashAccounts<'a, 'info>,
    input: SlashInput<'b, 'info>,
) -> Result<SlashOutcome> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let now_slot = clock.slot;

    let batch = slash_guilty_nodes(
        input.remaining_accounts,
        input.resolved,
        &input.guilty_bitmap,
        input.offence,
        now,
        accounts.protocol_config.withdrawal_cooldown_secs,
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

    let bounty = bps_of(
        slashed_total,
        accounts.protocol_config.challenger_bounty_bps,
    );
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

    Ok(SlashOutcome {
        slashed_total,
        bounty_paid: bounty,
        previous_version,
        node_count_after,
        redundancy_buffer_after,
        now,
        now_slot,
    })
}

/// Message hash a statement aggregate must verify over.
pub(crate) fn statement_message_hash(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: u64,
    statement: &SignedStatement,
) -> [u8; 32] {
    molpha_verifier::compute_message_hash(
        &molpha_verifier::AttestationPayload {
            source_id: *source_id,
            registry_version,
            value: statement.value,
            canonical_timestamp,
            signatures_required,
        },
        statement.signers_bitmap,
    )
}

/// Verify one signed statement against its resolved signers.
pub(crate) fn verify_statement(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: u64,
    statement: &SignedStatement,
    ordered_signers: &[molpha_verifier::SignerXy],
) -> Result<()> {
    let message_hash = statement_message_hash(
        source_id,
        registry_version,
        signatures_required,
        canonical_timestamp,
        statement,
    );
    let is_valid = molpha_verifier::verify_aggregate_over_hash(
        ordered_signers,
        &statement.agg_sig_s,
        &statement.commitment_addr,
        &message_hash,
    )
    .map_err(map_attestation_err)?;
    require!(is_valid, MolphaError::InvalidAggregateSignature);
    Ok(())
}
