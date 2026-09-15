//! Permissionless finalization after the dispute window: credits `reward_per_signature` to each
//! signer's `claimable_rewards` using the round's pinned registry, against a solvent treasury.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use molpha_verifier::bitmap::{Bitmap, for_each_set_bit};
use crate::{
    error::MolphaError,
    events::{NodeLivelinessPenalized, RoundFinalized},
    state::{
        Gateway, GatewayStatus, Node, ProtocolConfig, Registry, RoundReceiptRecord,
        RoundRecordStatus, LIVELINESS_PENALTY_PER_MISS, LIVELINESS_RECOVERY_PER_SIGN,
        LIVELINESS_SCORE_MAX,
    },
};

pub fn finalize_round<'info>(ctx: Context<'info, FinalizeRound<'info>>) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let record = &ctx.accounts.round_record;
    require!(
        ctx.accounts.gateway.status != GatewayStatus::Slashed,
        MolphaError::GatewaySlashed
    );

    let now_slot = Clock::get()?.slot;

    require!(
        now_slot.saturating_sub(record.recorded_slot) > config.dispute_window_slots,
        MolphaError::RoundNotFinalizable
    );

    let registry = ctx.accounts.registry.load()?;
    credit_round_rewards(
        &mut ctx.accounts.round_record,
        &registry,
        &mut ctx.accounts.protocol_config,
        &ctx.accounts.treasury,
        ctx.remaining_accounts,
        ctx.program_id,
    )
}

/// Credit every signer and move a pending round into its terminal finalized state.
///
/// Crediting mints a claim on the treasury, so it is refused unless the treasury can back the
/// protocol's whole reward liability afterwards. A refused round stays `Pending` and can be
/// finalized once the treasury is topped up.
pub(crate) fn credit_round_rewards<'info>(
    record: &mut Account<'info, RoundReceiptRecord>,
    registry: &Registry,
    config: &mut Account<'info, ProtocolConfig>,
    treasury: &Account<'info, TokenAccount>,
    remaining_accounts: &'info [AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<()> {
    let credited = Bitmap::load(&record.signers_bitmap);
    let credited_count = credited.popcount() as u32;
    let missed = Bitmap::load(&record.selection_bitmap).subtract(&credited);
    let missed_count = missed.popcount() as u32;

    require!(
        remaining_accounts.len() == (credited_count + missed_count) as usize,
        MolphaError::InvalidMissedNodeAccount
    );

    let reward = record.reward_per_signature;
    let total = reward
        .checked_mul(u64::from(credited_count))
        .ok_or(MolphaError::TreasuryInsolvent)?;
    let liability_after = config
        .reward_liability
        .checked_add(total)
        .ok_or(MolphaError::TreasuryInsolvent)?;
    require!(
        treasury.amount >= liability_after,
        MolphaError::TreasuryInsolvent
    );

    let mut cursor = 0usize;
    for_each_set_bit(credited, |bit_pos| {
        let node_ai = &remaining_accounts[cursor];
        cursor += 1;

        let mut node: Account<Node> = Account::try_from(node_ai)?;
        require_keys_eq!(
            node_ai.key(),
            registry.node(bit_pos)?,
            MolphaError::InvalidCreditNodeAccount
        );

        node.claimable_rewards = node
            .claimable_rewards
            .checked_add(reward)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        node.liveliness_score = node
            .liveliness_score
            .saturating_add(LIVELINESS_RECOVERY_PER_SIGN)
            .min(LIVELINESS_SCORE_MAX);
        node.exit(program_id)?;

        Ok(())
    })?;

    let round_record = record.key();
    for_each_set_bit(missed, |bit_pos| {
        let node_ai = &remaining_accounts[cursor];
        cursor += 1;

        require_keys_eq!(
            node_ai.key(),
            registry.node(bit_pos)?,
            MolphaError::InvalidMissedNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(node_ai)?;

        node.liveliness_score = node
            .liveliness_score
            .saturating_sub(LIVELINESS_PENALTY_PER_MISS);
        emit!(NodeLivelinessPenalized {
            node: node.key(),
            round_record,
            score: node.liveliness_score,
        });
        node.exit(program_id)?;

        Ok(())
    })?;

    config.reward_liability = liability_after;
    record.status = RoundRecordStatus::Finalized;

    let clock = Clock::get()?;
    emit!(RoundFinalized {
        round_record: record.key(),
        finalized_slot: clock.slot,
        finalized_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct FinalizeRound<'info> {
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// Read only: crediting checks the treasury can back the liability it is about to add.
    #[account(
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, round_record.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    /// Settlement record created by `settle_*_round`; seeds omitted because only those instructions init it.
    #[account(
        mut,
        constraint = round_record.status == RoundRecordStatus::Pending @ MolphaError::RoundNotPending,
    )]
    pub round_record: Account<'info, RoundReceiptRecord>,

    #[account(
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Account<'info, Gateway>,
}
