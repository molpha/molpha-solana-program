use anchor_lang::prelude::*;

use crate::{
    events::PlanAdded,
    state::{Plan, PlanInitParams, PlanType, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddPlanArgs {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_signers: u8,
    pub max_delegates: u32,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

pub fn add_plan(ctx: Context<AddPlan>, args: AddPlanArgs) -> Result<()> {
    ctx.accounts.plan.init(&PlanInitParams {
        plan_type: args.plan_type,
        subscription_price: args.subscription_price,
        max_signers: args.max_signers,
        max_delegates: args.max_delegates,
        private_api_enabled: args.private_api_enabled,
        is_active: args.is_active,
        bump: ctx.bumps.plan,
    });

    emit!(PlanAdded {
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
#[instruction(args: AddPlanArgs)]
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
        seeds = [Plan::SEED_PREFIX, &[args.plan_type as u8]],
        bump
    )]
    pub plan: Account<'info, Plan>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}
