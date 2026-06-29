use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::JobCreated,
    state::{Job, JobInitParams, Plan, ProtocolConfig, Subscription},
    utils::registry::derive_job_id,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateJobArgs {
    pub api_config_hash: [u8; 32],
    pub signatures_required: u8,
    pub decimals: u8,
}

pub fn create_job(ctx: Context<CreateJob>, args: CreateJobArgs, job_id: [u8; 32]) -> Result<()> {
    let plan = &ctx.accounts.plan;
    let creator = ctx.accounts.creator.key();

    require_keys_eq!(
        ctx.accounts.subscription.owner,
        creator,
        MolphaError::UnauthorisedJobOwner
    );

    let clock = Clock::get()?;

    let expected_job_id = derive_job_id(&creator, &args.api_config_hash, args.signatures_required);
    require!(job_id == expected_job_id, MolphaError::InvalidJobId);

    require!(
        args.signatures_required <= plan.max_signers,
        MolphaError::SignaturesExceedPlanMax
    );

    ctx.accounts.job.init(&JobInitParams {
        job_id,
        owner: creator,
        api_config_hash: args.api_config_hash,
        decimals: args.decimals,
        signatures_required: args.signatures_required,
        created_at: clock.unix_timestamp,
        bump: ctx.bumps.job,
    });

    ctx.accounts.subscription.job_count = ctx
        .accounts
        .subscription
        .job_count
        .checked_add(1)
        .ok_or(MolphaError::JobLimitReached)?;

    emit!(JobCreated {
        job_id,
        owner: creator,
        plan_type: plan.plan_type as u8,
        signatures_required: args.signatures_required,
        created_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: CreateJobArgs, job_id: [u8; 32])]
pub struct CreateJob<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [Plan::SEED_PREFIX, &[subscription.plan_type as u8]],
        bump = plan.bump,
        constraint = plan.is_active @ MolphaError::PlanNotActive,
    )]
    pub plan: Account<'info, Plan>,

    #[account(
        init,
        payer = creator,
        space = Job::SPACE,
        seeds = [Job::SEED_PREFIX, job_id.as_ref()],
        bump
    )]
    pub job: Account<'info, Job>,

    #[account(
        mut,
        seeds = [Subscription::SEED_PREFIX, creator.key().as_ref()],
        bump = subscription.bump,
        constraint = subscription.is_active() @ MolphaError::SubscriptionNotActive,
        constraint = subscription.job_count < plan.max_jobs @ MolphaError::JobLimitReached,
    )]
    pub subscription: Account<'info, Subscription>,

    pub system_program: Program<'info, System>,
}
