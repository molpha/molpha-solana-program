//! Shared node punishment: apply penalties, evict victims, and run the common sentence tail.
//!
//! A filing settles in one instruction. Because every node's principal sits in one shared vault,
//! the batch sweeps its whole forfeiture with a single transfer and funds the reporter's bounty
//! from it immediately, so the transaction costs one account per signer and nothing per guilty
//! node.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount, Transfer};
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
    /// Principal deducted from the victims and swept into the treasury.
    pub slashed_locked: u64,
    /// Reward credits cancelled, and so released from `ProtocolConfig.reward_liability`.
    pub slashed_rewards: u64,
}

impl PunishmentBatch {
    pub fn slashed_total(&self) -> u64 {
        self.slashed_locked.saturating_add(self.slashed_rewards)
    }
}

/// Slash every guilty node, accumulating what the batch forfeits.
///
/// Already-tombstoned nodes are skipped so one stale bit cannot block the batch. Nothing else is
/// skipped: the whole sentence settles in this instruction, so there is no half-settled state a
/// later offence could collide with, and no way for a node to become temporarily unpunishable.
fn slash_guilty_nodes<'info>(
    input: &SlashInput<'_, 'info>,
    withdrawal_cooldown_slots: u64,
) -> Result<PunishmentBatch> {
    let penalty = input.offence.penalty();
    let mut batch = PunishmentBatch::default();
    let clock = Clock::get()?;

    for ((bit_pos, _), account_info) in input.resolved.iter().zip(input.signer_accounts.iter()) {
        if !bitmap_bit_set(&input.guilty_bitmap, *bit_pos) {
            continue;
        }
        require!(
            account_info.owner == input.program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(account_info)?;
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
        node.exit(input.program_id)?;
    }

    Ok(batch)
}

#[allow(clippy::too_many_arguments)]
pub fn penalize_node<'info>(
    current_registry: &AccountLoader<'info, Registry>,
    new_registry: &mut AccountLoader<'info, Registry>,
    registry_state: &mut Account<'info, RegistryState>,
    node: &mut Account<'info, Node>,
    offence: OffenceKind,
    withdrawal_cooldown_slots: u64,
    new_registry_bump: u8,
    clock: &Clock,
) -> Result<PenaltyOutcome> {
    let outcome = apply_penalty(node, offence.penalty(), clock.unix_timestamp)?;
    if node.status == NodeStatus::Tombstoned {
        node.withdrawable_slot = clock
            .slot
            .checked_add(withdrawal_cooldown_slots)
            .ok_or(MolphaError::WithdrawalArithmeticOverflow)?;
    }

    let current = current_registry.load()?;
    let mut next = new_registry.load_init()?;
    next.swap_remove(&current, node.key(), new_registry_bump)?;
    registry_state.advance_version();

    emit!(NodePunished {
        node: node.key(),
        owner: node.owner,
        offence,
        slashed_locked: outcome.slashed_locked,
        slashed_rewards: outcome.slashed_rewards,
        new_status: node.status,
        punished_at: clock.unix_timestamp,
    });
    Ok(outcome)
}

/// Mutable accounts shared by every node-punishment sentence.
///
/// The token accounts are a fixed cost: one shared deposit vault however many nodes are guilty.
pub struct SlashAccounts<'a, 'info> {
    pub registry_state: &'a mut Account<'info, RegistryState>,
    pub current_registry: &'a AccountLoader<'info, Registry>,
    pub new_registry: &'a AccountLoader<'info, Registry>,
    pub new_registry_bump: u8,
    pub protocol_config: &'a mut Account<'info, ProtocolConfig>,
    pub punishment_case: &'a mut Account<'info, PunishmentCase>,
    pub punishment_case_bump: u8,
    pub reporter: Pubkey,
    // pub node_deposits: AccountInfo<'info>,
    pub node_deposit_vault: &'a mut Account<'info, TokenAccount>,
    pub treasury: &'a mut Account<'info, TokenAccount>,
    pub reporter_usdc: &'a Account<'info, TokenAccount>,
    pub token_program: Pubkey,
}

/// Proof output from an offence-specific handler, fed into [`slash_nodes`].
pub struct SlashInput<'a, 'info> {
    /// The signer `Node` accounts the aggregate was verified against, in ascending bit order.
    pub signer_accounts: &'info [AccountInfo<'info>],
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
}

/// Shared sentence: slash guilty nodes, evict them into a successor registry, pay the reporter
/// bounty, init `PunishmentCase`, and emit `RegistryVersionCreated`.
pub fn slash_nodes<'a, 'b, 'info>(
    accounts: SlashAccounts<'a, 'info>,
    input: SlashInput<'b, 'info>,
) -> Result<SlashOutcome> {
    let clock = Clock::get()?;

    let batch = slash_guilty_nodes(&input, accounts.protocol_config.withdrawal_cooldown_slots)?;
    require!(!batch.victims.is_empty(), MolphaError::NothingToPunish);
    let slashed_total = batch.slashed_total();

    let current = accounts.current_registry.load()?;
    let mut next = accounts.new_registry.load_init()?;
    next.swap_remove_batch(&current, &batch.victims, accounts.new_registry_bump)?;
    accounts.registry_state.advance_version();

    // Cancelled reward credits stop being a claim on the treasury.
    accounts.protocol_config.reward_liability = accounts
        .protocol_config
        .reward_liability
        .checked_sub(batch.slashed_rewards)
        .ok_or(MolphaError::RewardLiabilityUnderflow)?;

    // Sweep the forfeited principal out of the deposit vault in one transfer, whatever the size
    // of the guilty set. The amount comes directly from the guilty nodes' locked balances.
    if batch.slashed_locked > 0 {
        require!(
            accounts.node_deposit_vault.amount >= batch.slashed_locked,
            MolphaError::DepositVaultUnderfunded
        );

        let registry_state_seeds: &[&[u8]] =
            &[RegistryState::SEED_PREFIX, &[accounts.registry_state.bump]];
        token::transfer(
            CpiContext::new_with_signer(
                accounts.token_program,
                Transfer {
                    from: accounts.node_deposit_vault.to_account_info(),
                    to: accounts.treasury.to_account_info(),
                    authority: accounts.registry_state.to_account_info(),
                },
                &[registry_state_seeds],
            ),
            batch.slashed_locked,
        )?;
    }

    // The bounty is funded by what this punishment just recovered: the principal swept in above,
    // plus the reward credits released from `reward_liability`. `challenger_bounty_bps` is capped
    // at 100% of their sum, so it can never be paid out of another party's money.
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

        accounts.treasury.reload()?;
        require!(
            accounts.treasury.amount >= accounts.protocol_config.reward_liability,
            MolphaError::TreasuryInsolvent
        );
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
        clock.slot,
        accounts.punishment_case_bump,
    );

    Ok(SlashOutcome {
        slashed_total,
        bounty_paid: bounty,
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
