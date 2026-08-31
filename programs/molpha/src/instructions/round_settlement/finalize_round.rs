//! Permissionless finalization after the dispute window: credits `reward_per_signature` (and
//! to node/gateway `claimable_rewards` using the round's pinned registry.

use anchor_lang::prelude::*;
use molpha_verifier::bitmap::{bitmap_load, for_each_set_bit_u256};

use crate::{
    error::MolphaError,
    events::RoundFinalized,
    state::{
        Gateway, GatewayStatus, Node, ProtocolConfig, Registry, RoundReceiptRecord,
        RoundRecordStatus,
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
        ctx.remaining_accounts,
        ctx.program_id,
    )
}

/// Credit every signer and move a pending round into its terminal finalized state.
pub(crate) fn credit_round_rewards<'info>(
    record: &mut Account<'info, RoundReceiptRecord>,
    registry: &Registry,
    remaining_accounts: &'info [AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<()> {
    let credited = bitmap_load(&record.signers_bitmap);
    let credited_count = credited.count_ones();
    require!(
        remaining_accounts.len() == credited_count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    let reward = record.reward_per_signature;

    let mut cursor = 0usize;
    for_each_set_bit_u256(credited, |bit_pos| {
        let node_ai = remaining_accounts
            .get(cursor)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
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
        node.exit(program_id)?;

        Ok(())
    })?;

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
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

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
