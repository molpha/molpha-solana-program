use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::ProtocolConfigUpdated,
    state::{ProtocolConfig, MAX_REGISTRY_NODES},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateConfigArgs {
    pub withdrawal_cooldown_secs: i64,
    pub max_node_count: u32,
    pub minimum_node_deposit: u64,
    pub reward_per_signature: u64,
    pub max_receipt_delay_slots: u64,
    pub protocol_fee: u64,
    pub dispute_window_slots: u64,
    pub gateway_bond_min: u64,
    pub challenger_bounty_bps: u16,
    pub x402_round_base: u64,
    pub x402_round_per_sig: u64,
}

pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    require!(
        args.max_node_count > 0 && args.max_node_count <= MAX_REGISTRY_NODES as u32,
        MolphaError::NodeCapExceeded
    );

    let protocol_config = &mut ctx.accounts.protocol_config;
    protocol_config.reward_per_signature = args.reward_per_signature;
    protocol_config.max_receipt_delay_slots = args.max_receipt_delay_slots;
    protocol_config.protocol_fee = args.protocol_fee;
    protocol_config.dispute_window_slots = args.dispute_window_slots;
    protocol_config.gateway_bond_min = args.gateway_bond_min;
    protocol_config.challenger_bounty_bps = args.challenger_bounty_bps;
    protocol_config.max_node_count = args.max_node_count;
    protocol_config.minimum_node_deposit = args.minimum_node_deposit;
    protocol_config.withdrawal_cooldown_secs = args.withdrawal_cooldown_secs;
    protocol_config.x402_round_base = args.x402_round_base;
    protocol_config.x402_round_per_sig = args.x402_round_per_sig;

    emit!(ProtocolConfigUpdated {
        reward_per_signature: args.reward_per_signature,
        max_receipt_delay_slots: args.max_receipt_delay_slots,
        protocol_fee: args.protocol_fee,
        dispute_window_slots: args.dispute_window_slots,
        gateway_bond_min: args.gateway_bond_min,
        challenger_bounty_bps: args.challenger_bounty_bps,
        max_node_count: args.max_node_count,
        minimum_node_deposit: args.minimum_node_deposit,
        withdrawal_cooldown_secs: args.withdrawal_cooldown_secs,
        x402_round_base: args.x402_round_base,
        x402_round_per_sig: args.x402_round_per_sig,
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
#[cfg(test)]
mod tests {
    use super::UpdateConfigArgs;
    use anchor_lang::InstructionData;

    fn fixture_args() -> UpdateConfigArgs {
        UpdateConfigArgs {
            withdrawal_cooldown_secs: 3,
            max_node_count: 8,
            minimum_node_deposit: 100_000_000,
            reward_per_signature: 10,
            max_receipt_delay_slots: 256,
            protocol_fee: 30,
            dispute_window_slots: 1024,
            gateway_bond_min: 500_000_000,
            challenger_bounty_bps: 500,
            x402_round_base: 1_000,
            x402_round_per_sig: 15,
        }
    }

    #[test]
    fn update_config_args_roundtrip() {
        let args = fixture_args();
        let bytes = borsh::to_vec(&args).expect("serialize");
        let decoded = borsh::from_slice::<UpdateConfigArgs>(&bytes).expect("deserialize");

        assert_eq!(decoded.max_node_count, args.max_node_count);
        assert_eq!(decoded.minimum_node_deposit, args.minimum_node_deposit);
        assert_eq!(decoded.reward_per_signature, args.reward_per_signature);
        assert_eq!(
            decoded.max_receipt_delay_slots,
            args.max_receipt_delay_slots
        );
        assert_eq!(decoded.x402_round_base, args.x402_round_base);
        assert_eq!(decoded.x402_round_per_sig, args.x402_round_per_sig);
    }

    #[test]
    fn update_config_instruction_data_serializes_args() {
        let args = fixture_args();
        let data = crate::instruction::UpdateConfig { args: args.clone() }.data();
        assert_eq!(data.len(), 104);

        let decoded = borsh::from_slice::<UpdateConfigArgs>(&data[8..]).expect("deserialize args");
        assert_eq!(decoded.max_node_count, args.max_node_count);
        assert_eq!(decoded.x402_round_per_sig, args.x402_round_per_sig);
    }
}
