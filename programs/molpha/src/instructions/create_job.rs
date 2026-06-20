use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::JobCreated,
    state::{Delegate, Feed, Job, JobInitParams, Plan, ProtocolConfig, Subscription},
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
    let job_owner = ctx.accounts.job_owner.key();
    let creator = ctx.accounts.creator.key();

    require_keys_eq!(
        ctx.accounts.subscription.owner,
        job_owner,
        MolphaError::UnauthorisedJobOwner
    );

    let clock = Clock::get()?;

    let expected_job_id = derive_job_id(&job_owner, &args.api_config_hash);
    require!(job_id == expected_job_id, MolphaError::InvalidJobId);

    require!(
        args.signatures_required <= plan.max_signers,
        MolphaError::SignaturesExceedPlanMax
    );

    if creator != job_owner {
        let delegate_ai = ctx
            .accounts
            .delegate
            .as_ref()
            .ok_or(MolphaError::DelegateAccountRequired)?;
        let (expected_pda, _bump) = Pubkey::find_program_address(
            &[Delegate::SEED_PREFIX, job_owner.as_ref(), creator.as_ref()],
            ctx.program_id,
        );
        require_keys_eq!(
            delegate_ai.key(),
            expected_pda,
            MolphaError::UnauthorisedConsumer
        );

        let mut data = delegate_ai.try_borrow_mut_data()?;
        let mut delegate = Delegate::try_deserialize(&mut &data[..])?;
        require_keys_eq!(delegate.owner, job_owner, MolphaError::UnauthorisedConsumer);
        require_keys_eq!(
            delegate.delegate,
            creator,
            MolphaError::UnauthorisedConsumer
        );
        delegate.try_consume_job()?;
        delegate.try_serialize(&mut &mut data[..])?;
    } else {
        require!(
            ctx.accounts.delegate.is_none(),
            MolphaError::UnexpectedDelegateAccount
        );
    }

    ctx.accounts.job.init(&JobInitParams {
        job_id,
        owner: job_owner,
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

    ctx.accounts.feed.init(job_id, ctx.bumps.feed);

    emit!(JobCreated {
        job_id,
        owner: job_owner,
        creator,
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

    /// CHECK: validated against subscription.owner and job_id derivation.
    pub job_owner: UncheckedAccount<'info>,

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
        seeds = [Subscription::SEED_PREFIX, job_owner.key().as_ref()],
        bump = subscription.bump,
        constraint = subscription.is_active() @ MolphaError::SubscriptionNotActive,
        constraint = subscription.job_count < plan.max_jobs @ MolphaError::JobLimitReached,
    )]
    pub subscription: Account<'info, Subscription>,

    /// Required when `creator != job_owner`; enforces delegate job-creation quota.
    #[account(mut)]
    pub delegate: Option<UncheckedAccount<'info>>,

    #[account(
        init,
        payer = creator,
        space = Feed::SPACE,
        seeds = [Feed::SEED_PREFIX, job_id.as_ref()],
        bump
    )]
    pub feed: Account<'info, Feed>,

    pub system_program: Program<'info, System>,
}
