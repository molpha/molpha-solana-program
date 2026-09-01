use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::ProtocolConfigUpdated,
    state::{ProtocolConfig, MIN_SIGNERS_FLOOR},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateConfigArgs {
    pub withdrawal_cooldown_slots: u64,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_receipt_delay_seconds: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub x402_round_base: u64,
    pub x402_round_per_sig: u64,
    pub min_signers: u8,
}

pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    require!(
        args.withdrawal_cooldown_slots > 0,
        MolphaError::InvalidWithdrawalCooldown
    );
    require!(
        args.max_receipt_delay_seconds > 0,
        MolphaError::InvalidMaxReceiptDelay
    );
    require!(
        args.min_signers >= MIN_SIGNERS_FLOOR,
        MolphaError::InvalidMinSigners
    );

    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_receipt_delay_seconds = args.max_receipt_delay_seconds;
    protocol_config.dispute_window_slots = args.dispute_window_slots;
    protocol_config.gateway_bond_min = args.gateway_bond_min;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_slots = args.withdrawal_cooldown_slots;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.x402_round_per_sig = args.x402_round_per_sig;
    protocol_config.min_signers = args.min_signers;

    emit!(ProtocolConfigUpdated {
        reward_per_signature: args.reward_per_signature,
        max_receipt_delay_seconds: args.max_receipt_delay_seconds,
        dispute_window_slots: args.dispute_window_slots,
        gateway_bond_min: args.gateway_bond_min,
        challenger_bounty_bps: args.challenger_bounty_bps,
        minimum_node_deposit: args.minimum_node_deposit,
        withdrawal_cooldown_slots: args.withdrawal_cooldown_slots,
        x402_round_base: args.x402_round_base,
        x402_round_per_sig: args.x402_round_per_sig,
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
