//! `finalize_round` — permissionless finalization of round credits past the dispute window.
//!
//! After `dispute_window_slots` have elapsed and no successful dispute voided the round, this
//! credits each node's `reward_per_signature` (and the gateway's `gateway_fee`) to
//! `claimable_rewards`, after which `claim_*_rewards` may withdraw it.
//!
//! Credited nodes are resolved through the immutable registry version stored in the round record.
//! Permanent Node accounts can therefore be credited even after later membership changes.

use anchor_lang::prelude::*;
use molpha_verifier::bitmap::{bitmap_load, for_each_set_bit_u256};

use crate::{
    error::MolphaError,
    events::RoundFinalized,
    state::{Gateway, Node, ProtocolConfig, Registry, RoundReceiptRecord, RoundRecordStatus},
};

pub fn finalize_round<'info>(ctx: Context<FinalizeRound<'info>>) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let record = &ctx.accounts.round_record;
    let registry = ctx.accounts.registry.load()?;

    let clock = Clock::get()?;
    let now_slot = clock.slot;

    require!(
        record.status == RoundRecordStatus::Pending,
        MolphaError::RoundNotPending
    );
    require!(
        now_slot.saturating_sub(record.recorded_slot) > config.dispute_window_slots,
        MolphaError::RoundNotFinalizable
    );

    require!(
        record.registry_version == registry.version,
        MolphaError::InvalidRegistryVersion
    );

    let credited = bitmap_load(&record.signers_bitmap);
    let credited_count = credited.count_ones();
    require!(
        ctx.remaining_accounts.len() == credited_count as usize,
        MolphaError::InvalidCreditNodeAccount
    );

    let reward = record.reward_per_signature;
    let program_id = ctx.program_id;

    let mut cursor = 0usize;
    for_each_set_bit_u256(credited, |bit_pos| {
        let node_ai = ctx
            .remaining_accounts
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

    let gateway_fee = record.gateway_fee;
    let gateway = &mut ctx.accounts.gateway;
    gateway.claimable_rewards = gateway
        .claimable_rewards
        .checked_add(gateway_fee)
        .ok_or(error!(MolphaError::InvalidCreditNodeAccount))?;

    let record = &mut ctx.accounts.round_record;
    record.status = RoundRecordStatus::Finalized;

    emit!(RoundFinalized {
        round_record: record.key(),
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
        seeds = [Registry::SEED_PREFIX, round_record.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    /// Subscription (`["molpha_round", subscription.owner, source_id, sig_req, ts]`) or x402
    /// (`["molpha_x402_round", payer_id, gateway, source_id, sig_req, ts]`) settlement record.
    /// No seeds constraint: owner + discriminator suffice — only the two settle instructions can
    /// create this account type, and both write complete state, so stored contents are trusted.
    #[account(mut)]
    pub round_record: Account<'info, RoundReceiptRecord>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Account<'info, Gateway>,
}
