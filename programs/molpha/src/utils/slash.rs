//! Shared node punishment: apply penalties, evict victims, and run the common sentence tail.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::get_associated_token_address,
    token::{self, TokenAccount, Transfer},
};
use molpha_verifier::bitmap::bitmap_bit_set;

use crate::{
    error::{map_attestation_err, MolphaError},
    events::NodePunished,
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

/// Outcome of a [`slash_guilty_nodes`] batch.
#[derive(Clone, Debug, Default)]
pub struct PunishmentBatch {
    /// Nodes slashed in resolution order; may be shorter than `guilty_bitmap` if some are tombstoned.
    pub victims: Vec<Pubkey>,
    /// Principal swept out of the victims' deposit vaults into the treasury.
    pub slashed_locked: u64,
    /// Reward credits cancelled, and so released from `ProtocolConfig.reward_liability`.
    pub slashed_rewards: u64,
}

impl PunishmentBatch {
    pub fn slashed_total(&self) -> u64 {
        self.slashed_locked.saturating_add(self.slashed_rewards)
    }
}

/// Slash every guilty node and sweep the slashed principal out of its deposit vault into the
/// treasury, so the reporter bounty is funded by the offence rather than by other nodes' money.
///
/// `deposit_atas` carries one vault per set bit of `guilty_bitmap`, in ascending bit order. The
/// cursor advances for every guilty bit — including bits skipped because the node is already
/// tombstoned — so the caller's layout does not depend on on-chain node state.
///
/// Already-tombstoned nodes are skipped so one stale bit cannot block the batch.
fn slash_guilty_nodes<'a, 'info>(
    accounts: &SlashAccounts<'a, 'info>,
    input: &SlashInput<'_, 'info>,
    withdrawal_cooldown_slots: u64,
) -> Result<PunishmentBatch> {
    let penalty = input.offence.penalty();
    let mut batch = PunishmentBatch::default();
    let clock = Clock::get()?;

    require!(
        input.deposit_atas.len() == usize::from(input.guilty_count),
        MolphaError::InvalidNodeDepositAccount
    );

    let mut guilty_cursor = 0usize;
    for ((bit_pos, _), account_info) in input.resolved.iter().zip(input.signer_accounts.iter()) {
        if !bitmap_bit_set(&input.guilty_bitmap, *bit_pos) {
            continue;
        }
        let deposit_ata = input
            .deposit_atas
            .get(guilty_cursor)
            .ok_or(error!(MolphaError::InvalidNodeDepositAccount))?;
        guilty_cursor += 1;

        require!(
            account_info.owner == input.program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        require_keys_eq!(
            deposit_ata.key(),
            get_associated_token_address(&account_info.key(), &accounts.usdc_mint),
            MolphaError::InvalidNodeDepositAccount
        );

        let (node_owner, node_bump, slashed_locked) = {
            let mut node_data = account_info.try_borrow_mut_data()?;
            let mut node = Node::try_deserialize(&mut &node_data[..])?;
            if node.status == NodeStatus::Tombstoned {
                continue;
            }
            let outcome = apply_penalty(&mut node, penalty, clock.unix_timestamp)?;
            if node.status == NodeStatus::Tombstoned {
                node.withdrawable_slot = clock
                    .slot
                    .checked_add(withdrawal_cooldown_slots)
                    .ok_or(MolphaError::WithdrawalArithmeticOverflow)?;
            }
            batch.slashed_locked = batch.slashed_locked.saturating_add(outcome.slashed_locked);
            batch.slashed_rewards = batch
                .slashed_rewards
                .saturating_add(outcome.slashed_rewards);

            emit!(NodePunished {
                node: account_info.key(),
                owner: node.owner,
                offence: input.offence,
                slashed_locked: outcome.slashed_locked,
                slashed_rewards: outcome.slashed_rewards,
                new_status: node.status,
                punished_at: clock.unix_timestamp,
            });
            batch.victims.push(account_info.key());

            node.try_serialize(&mut &mut node_data[..])?;
            let node_bump = [node.bump];
            (node.owner, node_bump, outcome.slashed_locked)
        };

        // `node_deposit.amount == node.locked_amount` before the penalty, and `slash_bps` is
        // capped at 100%, so the vault always covers what was just slashed off the account.
        if slashed_locked > 0 {
            let signer_seeds: &[&[u8]] = &[Node::SEED_PREFIX, node_owner.as_ref(), &node_bump];
            token::transfer(
                CpiContext::new_with_signer(
                    accounts.token_program,
                    Transfer {
                        from: deposit_ata.clone(),
                        to: accounts.treasury.to_account_info(),
                        authority: account_info.clone(),
                    },
                    &[signer_seeds],
                ),
                slashed_locked,
            )?;
        }
    }

    Ok(batch)
}

/// Evict `victims` into a fresh registry snapshot. Rescans by pubkey because `swap_remove` shifts indices.
pub fn evict_nodes(
    current: &Registry,
    next: &mut Registry,
    bump: u8,
    victims: &[Pubkey],
) -> Result<()> {
    next.copy_from_previous(current, bump);
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
    pub protocol_config: &'a mut Account<'info, ProtocolConfig>,
    pub punishment_case: &'a mut Account<'info, PunishmentCase>,
    pub punishment_case_bump: u8,
    pub reporter: Pubkey,
    pub treasury: &'a mut Account<'info, TokenAccount>,
    pub reporter_usdc: &'a Account<'info, TokenAccount>,
    pub usdc_mint: Pubkey,
    pub token_program: Pubkey,
}

/// Proof output from an offence-specific handler, fed into [`slash_nodes`].
///
/// `remaining_accounts` is split by the handler: the signer `Node` accounts the aggregate was
/// verified against, then one deposit vault per set bit of `guilty_bitmap` in ascending order.
pub struct SlashInput<'a, 'info> {
    pub signer_accounts: &'info [AccountInfo<'info>],
    pub deposit_atas: &'info [AccountInfo<'info>],
    pub program_id: &'info Pubkey,
    pub resolved: &'a [(usize, molpha_verifier::SignerXy)],
    pub guilty_bitmap: [u8; 32],
    pub guilty_count: u16,
    pub offence: OffenceKind,
    pub registry_version: u32,
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

/// Split `remaining_accounts` into the signer `Node` accounts and the guilty deposit vaults.
pub fn split_punishment_accounts<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    signer_count: usize,
) -> Result<(&'info [AccountInfo<'info>], &'info [AccountInfo<'info>])> {
    require!(
        remaining_accounts.len() >= signer_count,
        MolphaError::InvalidCreditNodeAccount
    );
    Ok(remaining_accounts.split_at(signer_count))
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

    let withdrawal_cooldown_slots = accounts.protocol_config.withdrawal_cooldown_slots;
    let batch = slash_guilty_nodes(&accounts, &input, withdrawal_cooldown_slots)?;
    require!(!batch.victims.is_empty(), MolphaError::NothingToPunish);
    let slashed_total = batch.slashed_total();

    // Cancelled reward credits stop being a claim on the treasury.
    accounts.protocol_config.reward_liability = accounts
        .protocol_config
        .reward_liability
        .checked_sub(batch.slashed_rewards)
        .ok_or(MolphaError::RewardLiabilityUnderflow)?;

    let previous_version = accounts.registry_state.current_version;
    let (node_count_after, redundancy_buffer_after) = {
        let current = accounts.current_registry.load()?;
        let mut next = accounts.new_registry.load_init()?;
        evict_nodes(
            &current,
            &mut next,
            accounts.new_registry_bump,
            &batch.victims,
        )?;
        (next.node_count, next.redundancy_buffer)
    };
    accounts.registry_state.advance_version();

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

    // The bounty is at most 100% of what this punishment just recovered, and the principal leg
    // was swept in above, so this can only fail if the treasury was drained from elsewhere.
    accounts.treasury.reload()?;
    require!(
        accounts.treasury.amount >= accounts.protocol_config.reward_liability,
        MolphaError::TreasuryInsolvent
    );

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
