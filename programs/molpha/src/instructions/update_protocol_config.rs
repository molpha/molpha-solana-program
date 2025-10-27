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
