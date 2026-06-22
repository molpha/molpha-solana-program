//! `finalize_round` — permissionless promotion of credits past the dispute window.
//!
//! After `dispute_window_slots` have elapsed and no successful dispute voided the round, this
//! moves each credited node's `reward_per_signature` (and the gateway's `gateway_fee`) from
//! `pending_rewards` to `claimable_rewards`, after which `claim_*_rewards` may withdraw it.
//!
//! Credited nodes are verified against `RoundReceiptRecord.credited_nodes_hash`, committed at
//! settlement, so finalization does not depend on the live registry grace window.

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::RoundFinalized,
    state::{
        hash_credited_nodes, Gateway, Node, ProtocolConfig, RoundReceiptRecord,
    },
    utils::bitmap::{bitmap_load, for_each_set_bit_u256},
};

pub fn finalize_round<'info>(
    ctx: Context<'_, '_, 'info, 'info, FinalizeRound<'info>>,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let record = &ctx.accounts.round_record;

    require!(!record.voided, MolphaError::RoundVoided);
    require!(!record.finalized, MolphaError::RoundAlreadyFinalized);

    let clock = Clock::get()?;
    let now_slot = clock.slot;
    require!(
        now_slot.saturating_sub(record.recorded_slot) > config.dispute_window_slots,
        MolphaError::RoundNotFinalizable
    );

    let credited = bitmap_load(&record.credited_bitmap);
    let credited_count = credited.count_ones();
    require!(
        ctx.remaining_accounts.len() == credited_count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    let reward = record.reward_per_signature;
    let program_id = ctx.program_id;

    let mut credited_node_keys: Vec<Pubkey> = Vec::with_capacity(credited_count as usize);
    let mut cursor = 0usize;
    for_each_set_bit_u256(credited, |_bit_pos| -> Result<()> {
        let node_ai = ctx
            .remaining_accounts
            .get(cursor)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        cursor += 1;

        let mut node: Account<Node> = Account::try_from(node_ai)?;
        credited_node_keys.push(node.key());
        node.pending_rewards = node
            .pending_rewards
            .checked_sub(reward)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        node.claimable_rewards = node
            .claimable_rewards
            .checked_add(reward)
            .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
        node.exit(program_id)?;

        Ok(())
    })?;

    require!(
        hash_credited_nodes(&credited_node_keys) == record.credited_nodes_hash,
        MolphaError::CreditedNodesMismatch
    );

    let gateway_fee = record.gateway_fee;
    let gateway = &mut ctx.accounts.gateway;
    gateway.pending_rewards = gateway
        .pending_rewards
        .checked_sub(gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;
    gateway.claimable_rewards = gateway
        .claimable_rewards
        .checked_add(gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;

    let record = &mut ctx.accounts.round_record;
    record.finalized = true;

    emit!(RoundFinalized {
        job_id: record.job_id,
        timestamp: record.timestamp,
        credited_count,
        finalized_slot: now_slot,
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
        mut,
        seeds = [
            RoundReceiptRecord::SEED_PREFIX,
            round_record.job_id.as_ref(),
            round_record.timestamp.to_le_bytes().as_ref(),
        ],
        bump = round_record.bump,
    )]
    pub round_record: Account<'info, RoundReceiptRecord>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Account<'info, Gateway>,
}
