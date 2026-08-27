use anchor_lang::prelude::*;

use crate::{
    events::PlanUpdated,
    state::{Plan, PlanType, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdatePlanArgs {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_signers: u8,
    pub max_delegates: u32,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

pub fn update_plan(ctx: Context<UpdatePlan>, args: UpdatePlanArgs) -> Result<()> {
    ctx.accounts.plan.update(
        args.plan_type,
        args.subscription_price,
        args.max_signers,
        args.max_delegates,
        args.private_api_enabled,
        args.is_active,
    );

    emit!(PlanUpdated {
        plan_type: args.plan_type as u8,
        subscription_price: args.subscription_price,
        max_signers: args.max_signers,
        max_delegates: args.max_delegates,
        private_api_enabled: args.private_api_enabled,
        is_active: args.is_active,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdatePlan<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Plan::SEED_PREFIX, &[plan.plan_type as u8]],
        bump = plan.bump,
    )]
    pub plan: Account<'info, Plan>,
}
