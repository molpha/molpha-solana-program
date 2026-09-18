use anchor_lang::prelude::*;

use crate::{
    events::PlanUpdated,
    state::{Plan, PlanConfig, ProtocolConfig},
};

pub fn update_plan(ctx: Context<UpdatePlan>, config: PlanConfig) -> Result<()> {
    ctx.accounts.plan.update(&config);

    emit!(PlanUpdated {
        plan_type: ctx.accounts.plan.plan_type as u8,
        subscription_price: config.subscription_price,
        max_signers: config.max_signers,
        max_delegates: config.max_delegates,
        private_api_enabled: config.private_api_enabled,
        is_active: config.is_active,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(config: PlanConfig)]
pub struct UpdatePlan<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Plan::SEED_PREFIX, &[config.plan_type as u8]],
        bump = plan.bump,
    )]
    pub plan: Account<'info, Plan>,
}
