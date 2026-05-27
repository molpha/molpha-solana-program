use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::PlanUpdated,
    state::{Plan, PlanType, ProtocolConfig},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdatePlanArgs {
    pub plan_type: PlanType,
    pub subscription_price: u64,
    pub max_jobs: u32,
    pub max_signers: u8,
    pub private_api_enabled: bool,
    pub is_active: bool,
}

pub fn update_plan(ctx: Context<UpdatePlan>, args: UpdatePlanArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );

    ctx.accounts.plan.update(
        args.plan_type,
        args.subscription_price,
        args.max_jobs,
        args.max_signers,
        args.private_api_enabled,
        args.is_active,
    );

    emit!(PlanUpdated {
        plan_type: args.plan_type as u8,
        subscription_price: args.subscription_price,
        max_jobs: args.max_jobs,
        max_signers: args.max_signers,
        private_api_enabled: args.private_api_enabled,
        is_active: args.is_active,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdatePlan<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Plan::SEED_PREFIX, &[plan.plan_type as u8]],
        bump = plan.bump,
    )]
    pub plan: Account<'info, Plan>,

    pub authority: Signer<'info>,
}
