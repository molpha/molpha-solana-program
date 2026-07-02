//! `finalize_round` — permissionless promotion of credits past the dispute window.
//!
//! After `dispute_window_slots` have elapsed and no successful dispute voided the round, this
//! moves each credited node's `reward_per_signature` (and the gateway's `gateway_fee`) from
//! `pending_rewards` to `claimable_rewards`, after which `claim_*_rewards` may withdraw it.
//!
//! Credited nodes are re-identified through their slot `Node` accounts (applying the
//! previous-version-after-remove remap), so finalization must occur while the round's registry
//! version is still current or in grace — consistent with the rest of the protocol.

use anchor_lang::prelude::*;

use crate::{
    error::{map_data_update_err, MolphaError},
    events::RoundFinalized,
    state::{
        Gateway, Node, ProtocolConfig, RegistryState, RegistryTransitionType, RoundReceiptRecord,
    },
    utils::bitmap::{bitmap_load, for_each_set_bit_u256},
};

pub fn finalize_round<'info>(
    ctx: Context<'_, '_, 'info, 'info, FinalizeRound<'info>>,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let registry = &ctx.accounts.registry_state;
    let record = &ctx.accounts.round_record;

    require!(!record.voided, MolphaError::RoundVoided);
    require!(!record.finalized, MolphaError::RoundAlreadyFinalized);

    let clock = Clock::get()?;
    let now_slot = clock.slot;
    require!(
        now_slot.saturating_sub(record.recorded_slot) > config.dispute_window_slots,
        MolphaError::RoundNotFinalizable
    );

    let now_ts = clock.unix_timestamp;
    let is_current = record.registry_version == registry.current_version;
    let is_previous_live = record.registry_version == registry.previous_version
        && now_ts <= registry.previous_expires_at;
    require!(
        is_current || is_previous_live,
        MolphaError::InvalidRegistryVersion
    );
    let apply_remove_remap =
        !is_current && registry.last_transition_type != RegistryTransitionType::Add;
    let registry_view = registry.view();
    if apply_remove_remap {
        molpha_verifier::validate_remove_transition_for_previous(&registry_view)
            .map_err(map_data_update_err)?;
    }

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

        require!(
            node_ai.owner == program_id,
            MolphaError::InvalidCreditNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(node_ai)?;
        let expected_index = molpha_verifier::expected_node_index(
            bit_pos as u32,
            &registry_view,
            apply_remove_remap,
        );
        require!(node.index == expected_index, MolphaError::InvalidNodeIndex);

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
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

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
