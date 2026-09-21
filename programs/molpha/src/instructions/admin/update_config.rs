use anchor_lang::prelude::*;

use crate::{events::ProtocolConfigUpdated, state::ProtocolConfig};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateConfigArgs {
    pub withdrawal_cooldown_slots: u64,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_settlement_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub liveliness_freeze_threshold: u16,
    pub liveliness_penalty_per_miss: u16,
    pub liveliness_recovery_per_sign: u16,
    pub x402_round_base: u64,
    pub min_signers: u8,
}

pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_settlement_delay_seconds = args.max_settlement_delay_seconds;
    protocol_config.dispute_window_slots = args.dispute_window_slots;
    protocol_config.gateway_bond_min = args.gateway_bond_min;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_slots = args.withdrawal_cooldown_slots;
    protocol_config.liveliness_freeze_threshold = args.liveliness_freeze_threshold;
    protocol_config.liveliness_penalty_per_miss = args.liveliness_penalty_per_miss;
    protocol_config.liveliness_recovery_per_sign = args.liveliness_recovery_per_sign;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.min_signers = args.min_signers;
    protocol_config.validate()?;

    emit!(ProtocolConfigUpdated {
        reward_per_signature: args.reward_per_signature,
        max_settlement_delay_seconds: args.max_settlement_delay_seconds,
        dispute_window_slots: args.dispute_window_slots,
        gateway_bond_min: args.gateway_bond_min,
        challenger_bounty_bps: args.challenger_bounty_bps,
        minimum_node_deposit: args.minimum_node_deposit,
        withdrawal_cooldown_slots: args.withdrawal_cooldown_slots,
        liveliness_freeze_threshold: args.liveliness_freeze_threshold,
        liveliness_penalty_per_miss: args.liveliness_penalty_per_miss,
        liveliness_recovery_per_sign: args.liveliness_recovery_per_sign,
        x402_round_base: args.x402_round_base,
        min_signers: args.min_signers,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
}
