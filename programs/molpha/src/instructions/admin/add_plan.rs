use anchor_lang::prelude::*;

use crate::{
    events::PlanAdded,
    state::{Plan, PlanConfig, ProtocolConfig},
};

pub fn add_plan(ctx: Context<AddPlan>, config: PlanConfig) -> Result<()> {
    ctx.accounts.plan.init(&config, ctx.bumps.plan);

    emit!(PlanAdded {
        plan_type: config.plan_type as u8,
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
pub struct AddPlan<'info> {
    #[account(
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = authority,
        space = Plan::SPACE,
        seeds = [Plan::SEED_PREFIX, &[config.plan_type as u8]],
        bump
    )]
    pub plan: Account<'info, Plan>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}
