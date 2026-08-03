use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::ProtocolConfigUpdated,
    state::{ProtocolConfig, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateConfigArgs {
    pub registry_cooldown_secs: i64,
    pub previous_grace_secs: i64,
    pub withdrawal_cooldown_secs: i64,
    pub max_node_count: u32,
    pub redundancy_buffer: u8,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub gateway_fee: u64,
    pub receipt_submitter_bonus: u64,
    pub protocol_fee_bps: u16,
    pub max_receipt_delay_slots: u64,
    pub protocol_fee: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub agent_round_base: u64,
    pub agent_round_per_sig: u64,
    pub agent_epoch_len_secs: i64,
}

pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );
    require!(args.agent_epoch_len_secs > 0, MolphaError::InvalidAgentCaps);

    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.gateway_fee = args.gateway_fee;
    protocol_config.receipt_submitter_bonus = args.receipt_submitter_bonus;
    protocol_config.protocol_fee_bps = args.protocol_fee_bps;
    protocol_config.max_receipt_delay_slots = args.max_receipt_delay_slots;
    protocol_config.protocol_fee = args.protocol_fee;
    protocol_config.dispute_window_slots = args.dispute_window_slots;
    protocol_config.gateway_bond_min = args.gateway_bond_min;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.registry_cooldown_secs = args.registry_cooldown_secs;
    protocol_config.previous_grace_secs = args.previous_grace_secs;
    protocol_config.max_node_count = args.max_node_count;
    protocol_config.redundancy_buffer = args.redundancy_buffer;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_secs = args.withdrawal_cooldown_secs;
    protocol_config.agent_round_base = args.agent_round_base;
    protocol_config.agent_round_per_sig = args.agent_round_per_sig;
    protocol_config.agent_epoch_len_secs = args.agent_epoch_len_secs;

    let registry_state = &mut ctx.accounts.registry_state;
    registry_state.redundancy_buffer = args.redundancy_buffer;
    registry_state.grace_secs = args.previous_grace_secs;

    emit!(ProtocolConfigUpdated {
        reward_per_signature: args.reward_per_signature,
        gateway_fee: args.gateway_fee,
        receipt_submitter_bonus: args.receipt_submitter_bonus,
        protocol_fee_bps: args.protocol_fee_bps,
        max_receipt_delay_slots: args.max_receipt_delay_slots,
        protocol_fee: args.protocol_fee,
        dispute_window_slots: args.dispute_window_slots,
        gateway_bond_min: args.gateway_bond_min,
        challenger_bounty_bps: args.challenger_bounty_bps,
        registry_cooldown_secs: args.registry_cooldown_secs,
        previous_grace_secs: args.previous_grace_secs,
        max_node_count: args.max_node_count,
        redundancy_buffer: args.redundancy_buffer,
        minimum_node_deposit: args.minimum_node_deposit,
        withdrawal_cooldown_secs: args.withdrawal_cooldown_secs,
        agent_round_base: args.agent_round_base,
        agent_round_per_sig: args.agent_round_per_sig,
        agent_epoch_len_secs: args.agent_epoch_len_secs,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    pub authority: Signer<'info>,
}

#[cfg(test)]
mod tests {
    use super::UpdateConfigArgs;
    use anchor_lang::InstructionData;

    fn fixture_args() -> UpdateConfigArgs {
        UpdateConfigArgs {
            registry_cooldown_secs: 1,
            previous_grace_secs: 2,
            withdrawal_cooldown_secs: 3,
            max_node_count: 8,
            redundancy_buffer: 2,
            minimum_node_deposit: 100_000_000,
            reward_per_signature: 10,
            gateway_fee: 20,
            receipt_submitter_bonus: 5,
            protocol_fee_bps: 100,
            max_receipt_delay_slots: 256,
            protocol_fee: 30,
            dispute_window_slots: 1024,
            gateway_bond_min: 500_000_000,
            challenger_bounty_bps: 500,
            agent_round_base: 1_000,
            agent_round_per_sig: 15,
            agent_epoch_len_secs: 86_400,
        }
    }

    #[test]
    fn update_config_args_roundtrip() {
        let args = fixture_args();
        let bytes = borsh::to_vec(&args).expect("serialize");
        let decoded = borsh::from_slice::<UpdateConfigArgs>(&bytes).expect("deserialize");

        assert_eq!(decoded.registry_cooldown_secs, args.registry_cooldown_secs);
        assert_eq!(decoded.previous_grace_secs, args.previous_grace_secs);
        assert_eq!(decoded.max_node_count, args.max_node_count);
        assert_eq!(decoded.minimum_node_deposit, args.minimum_node_deposit);
        assert_eq!(decoded.reward_per_signature, args.reward_per_signature);
        assert_eq!(decoded.gateway_fee, args.gateway_fee);
        assert_eq!(
            decoded.receipt_submitter_bonus,
            args.receipt_submitter_bonus
        );
        assert_eq!(decoded.protocol_fee_bps, args.protocol_fee_bps);
        assert_eq!(
            decoded.max_receipt_delay_slots,
            args.max_receipt_delay_slots
        );
        assert_eq!(decoded.agent_round_base, args.agent_round_base);
        assert_eq!(decoded.agent_round_per_sig, args.agent_round_per_sig);
        assert_eq!(decoded.agent_epoch_len_secs, args.agent_epoch_len_secs);
    }

    #[test]
    fn update_config_instruction_data_serializes_args() {
        let args = fixture_args();
        let data = crate::instruction::UpdateConfig { args: args.clone() }.data();
        // Same payload size as initialize args minus the 32-byte usdc_mint pubkey.
        assert_eq!(data.len(), 137 + 24 - 32);

        let decoded = borsh::from_slice::<UpdateConfigArgs>(&data[8..]).expect("deserialize args");
        assert_eq!(decoded.max_node_count, args.max_node_count);
        assert_eq!(decoded.agent_epoch_len_secs, args.agent_epoch_len_secs);
    }
}
