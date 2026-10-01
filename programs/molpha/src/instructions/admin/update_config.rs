use anchor_lang::prelude::*;

use crate::{events::ProtocolConfigUpdated, state::ProtocolConfig};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateConfigArgs {
    pub withdrawal_cooldown_slots: u64,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_settlement_delay_seconds: u64,
    pub challenger_bounty_bps: u16,
    pub x402_round_base: u64,
    pub min_signers: u8,
    /// Epoch parameters apply to epochs created after this update, never to existing ones.
    pub epoch_len_seconds: u64,
    pub ticket_grace_seconds: u64,
    pub target_tickets_per_epoch: u32,
    pub min_availability_bps: u16,
    pub min_availability_samples: u16,
    pub protocol_fee_bps: u16,
}

pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_settlement_delay_seconds = args.max_settlement_delay_seconds;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_slots = args.withdrawal_cooldown_slots;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.min_signers = args.min_signers;
    protocol_config.epoch_len_seconds = args.epoch_len_seconds;
    protocol_config.ticket_grace_seconds = args.ticket_grace_seconds;
    protocol_config.target_tickets_per_epoch = args.target_tickets_per_epoch;
    protocol_config.min_availability_bps = args.min_availability_bps;
    protocol_config.min_availability_samples = args.min_availability_samples;
    protocol_config.protocol_fee_bps = args.protocol_fee_bps;
    protocol_config.validate()?;

    emit!(ProtocolConfigUpdated {
        reward_per_signature: args.reward_per_signature,
        max_settlement_delay_seconds: args.max_settlement_delay_seconds,
        challenger_bounty_bps: args.challenger_bounty_bps,
        minimum_node_deposit: args.minimum_node_deposit,
        withdrawal_cooldown_slots: args.withdrawal_cooldown_slots,
        x402_round_base: args.x402_round_base,
        min_signers: args.min_signers,
        epoch_len_seconds: args.epoch_len_seconds,
        ticket_grace_seconds: args.ticket_grace_seconds,
        target_tickets_per_epoch: args.target_tickets_per_epoch,
        min_availability_bps: args.min_availability_bps,
        min_availability_samples: args.min_availability_samples,
        protocol_fee_bps: args.protocol_fee_bps,
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
