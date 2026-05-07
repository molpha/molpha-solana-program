use crate::error::ProtocolConfigError;
use crate::events::ProtocolConfigUpdated;
use crate::state::ProtocolConfig;
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq)]
pub struct UpdateProtocolConfigParams {
    pub base_price_per_second_scaled: Option<u64>,
    pub frequency_coefficient: Option<u64>,
    pub signers_coefficient: Option<u64>,
    pub reward_percentage: Option<u64>,
    pub node_security_deposit: Option<u64>,

    // Wormhole cross-chain messaging configuration
    pub wormhole_bridge_program: Option<Pubkey>,
    pub target_chain_ids: Option<Vec<u16>>,
    pub registry_x_agg: Option<[u8; 33]>,
    pub signatures_required: Option<u8>,
    pub node_count: Option<u32>,
    pub redundancy_buffer: Option<u32>,
}

pub fn update_protocol_config(
    ctx: Context<UpdateProtocolConfig>,
    params: UpdateProtocolConfigParams,
) -> Result<()> {
    let clock = Clock::get()?;
    let protocol_config = &mut ctx.accounts.protocol_config;

    if let Some(base_price_per_second_scaled) = params.base_price_per_second_scaled {
        protocol_config.base_price_per_second_scaled = base_price_per_second_scaled;
    }

    if let Some(frequency_coefficient) = params.frequency_coefficient {
        protocol_config.frequency_coefficient = frequency_coefficient;
    }

    if let Some(signers_coefficient) = params.signers_coefficient {
        protocol_config.signers_coefficient = signers_coefficient;
    }

    if let Some(reward_percentage) = params.reward_percentage {
        protocol_config.reward_percentage = reward_percentage;
    }

    if let Some(node_security_deposit) = params.node_security_deposit {
        protocol_config.node_security_deposit = node_security_deposit;
    }

    // Update Wormhole configuration
    if let Some(wormhole_bridge_program) = params.wormhole_bridge_program {
        protocol_config.wormhole_bridge_program = wormhole_bridge_program;
    }

    if let Some(target_chain_ids) = params.target_chain_ids {
        require!(
            target_chain_ids.len() <= 10,
            ProtocolConfigError::TooManyTargetChains
        );
        protocol_config.target_chain_ids = target_chain_ids;
    }

    if let Some(registry_x_agg) = params.registry_x_agg {
        protocol_config.registry_x_agg = registry_x_agg;
    }

    if let Some(signatures_required) = params.signatures_required {
        protocol_config.signatures_required = signatures_required;
    }

    if let Some(node_count) = params.node_count {
        protocol_config.node_count = node_count;
    }

    if let Some(redundancy_buffer) = params.redundancy_buffer {
        protocol_config.redundancy_buffer = redundancy_buffer;
    }

    // Emit event
    emit!(ProtocolConfigUpdated {
        protocol_config: protocol_config.key(),
        authority: ctx.accounts.authority.key(),
        base_price_per_second_scaled: protocol_config.base_price_per_second_scaled,
        frequency_coefficient: protocol_config.frequency_coefficient,
        signers_coefficient: protocol_config.signers_coefficient,
        reward_percentage: protocol_config.reward_percentage,
        node_security_deposit: protocol_config.node_security_deposit,
        updated_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateProtocolConfig<'info> {
    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
        has_one = authority
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,
}
