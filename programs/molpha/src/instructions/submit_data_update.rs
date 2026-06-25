use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{FeedUpdated, JobCreated},
    instructions::data_update_core::build_coalition_and_verify,
    state::{Delegate, Feed, Job, JobInitParams, Plan, RegistryState, Subscription},
    utils::registry::derive_job_id,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitDataUpdateArgs {
    pub job_id: [u8; 32],
    pub signatures_required: u8,
    pub registry_version: u32,
    pub signers_bitmap: [u8; 32],
    pub value: [u8; 32],
    pub canonical_timestamp: i64,
    /// Schnorr signature R component
    pub agg_sig_s: [u8; 32],
    pub commitment_addr: [u8; 20],
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SubmitJobInitArgs {
    pub api_config_hash: [u8; 32],
    pub decimals: u8,
}

pub fn submit_data_update(
    mut ctx: Context<SubmitDataUpdate>,
    args: SubmitDataUpdateArgs,
    job_init: Option<SubmitJobInitArgs>,
) -> Result<()> {
    let clock = Clock::get()?;

    let job_is_new = ctx.accounts.job.owner == Pubkey::default();
    if job_is_new {
        init_job_if_needed(
            &mut ctx.accounts,
            &args,
            job_init.as_ref(),
            &clock,
            ctx.program_id,
            ctx.bumps.job,
        )?;
    } else {
        validate_existing_job(&ctx.accounts.job, &args, job_init.as_ref())?;
    }

    let feed = &mut ctx.accounts.feed;
    let feed_is_new = feed.job_id == [0u8; 32]
        && feed.registry_version == 0
        && feed.canonical_timestamp == 0
        && feed.value == [0u8; 32]
        && feed.signers_bitmap == [0u8; 32]
        && feed.signatures_required == 0
        && feed.last_updated_slot == 0;
    if feed_is_new {
        feed.init(args.job_id, ctx.bumps.feed);
    } else {
        require!(
            feed.job_id == args.job_id,
            MolphaError::DataUpdateJobMismatch
        );
        require!(
            feed.bump == ctx.bumps.feed,
            MolphaError::DataUpdateJobMismatch
        );
    }

    require!(
        args.signatures_required == ctx.accounts.job.signatures_required,
        MolphaError::DataUpdateConfigMismatch
    );

    let feed = &mut ctx.accounts.feed;
    let registry = &ctx.accounts.registry_state;

    require!(
        args.canonical_timestamp > feed.canonical_timestamp,
        MolphaError::FeedNotNewer
    );

    let now = clock.unix_timestamp;

    build_coalition_and_verify(
        &args,
        registry,
        now,
        ctx.remaining_accounts,
        ctx.program_id,
        args.signatures_required,
        registry.redundancy_buffer,
    )?;

    feed.value = args.value;
    feed.canonical_timestamp = args.canonical_timestamp;
    feed.registry_version = args.registry_version;
    feed.signers_bitmap = args.signers_bitmap;
    feed.signatures_required = args.signatures_required;
    feed.last_updated_slot = clock.slot;

    emit!(FeedUpdated {
        job_id: args.job_id,
        registry_version: args.registry_version,
        canonical_timestamp: args.canonical_timestamp,
        value: args.value,
        signers_bitmap: args.signers_bitmap,
        last_updated_slot: clock.slot,
    });

    Ok(())
}

fn init_job_if_needed(
    accounts: &mut SubmitDataUpdate<'_>,
    args: &SubmitDataUpdateArgs,
    job_init: Option<&SubmitJobInitArgs>,
    clock: &Clock,
    program_id: &Pubkey,
    job_bump: u8,
) -> Result<()> {
    let job_init = job_init.ok_or(MolphaError::DataUpdateConfigMismatch)?;
    let job_owner = accounts
        .job_owner
        .as_ref()
        .ok_or(MolphaError::UnauthorisedJobOwner)?
        .key();
    let plan = accounts.plan.as_ref().ok_or(MolphaError::PlanNotActive)?;
    let subscription = accounts
        .subscription
        .as_mut()
        .ok_or(MolphaError::SubscriptionNotActive)?;
    let creator = accounts.submitter.key();

    let expected_job_id = derive_job_id(
        &job_owner,
        &job_init.api_config_hash,
        args.signatures_required,
    );
    require!(args.job_id == expected_job_id, MolphaError::InvalidJobId);

    let (expected_subscription, _) =
        Pubkey::find_program_address(&[Subscription::SEED_PREFIX, job_owner.as_ref()], program_id);
    require_keys_eq!(
        subscription.key(),
        expected_subscription,
        MolphaError::UnauthorisedJobOwner
    );
    require_keys_eq!(
        subscription.owner,
        job_owner,
        MolphaError::UnauthorisedJobOwner
    );

    let (expected_plan, _) = Pubkey::find_program_address(
        &[Plan::SEED_PREFIX, &[subscription.plan_type as u8]],
        program_id,
    );
    require_keys_eq!(plan.key(), expected_plan, MolphaError::PlanNotActive);
    require!(plan.is_active, MolphaError::PlanNotActive);
    require!(subscription.is_active(), MolphaError::SubscriptionNotActive);
    require!(
        subscription.job_count < plan.max_jobs,
        MolphaError::JobLimitReached
    );
    require!(
        args.signatures_required <= plan.max_signers,
        MolphaError::SignaturesExceedPlanMax
    );

    if creator != job_owner {
        let delegate = accounts
            .delegate
            .as_mut()
            .ok_or(MolphaError::DelegateAccountRequired)?;
        let (expected_delegate, _) = Pubkey::find_program_address(
            &[Delegate::SEED_PREFIX, job_owner.as_ref(), creator.as_ref()],
            program_id,
        );
        require_keys_eq!(
            delegate.key(),
            expected_delegate,
            MolphaError::UnauthorisedConsumer
        );

        require_keys_eq!(delegate.owner, job_owner, MolphaError::UnauthorisedConsumer);
        require_keys_eq!(
            delegate.delegate,
            creator,
            MolphaError::UnauthorisedConsumer
        );
        delegate.try_consume_job()?;
    } else {
        require!(
            accounts.delegate.is_none(),
            MolphaError::UnexpectedDelegateAccount
        );
    }

    accounts.job.init(&JobInitParams {
        job_id: args.job_id,
        owner: job_owner,
        api_config_hash: job_init.api_config_hash,
        decimals: job_init.decimals,
        signatures_required: args.signatures_required,
        created_at: clock.unix_timestamp,
        bump: job_bump,
    });

    subscription.job_count = subscription
        .job_count
        .checked_add(1)
        .ok_or(MolphaError::JobLimitReached)?;

    emit!(JobCreated {
        job_id: args.job_id,
        owner: job_owner,
        creator,
        plan_type: plan.plan_type as u8,
        signatures_required: args.signatures_required,
        created_at: clock.unix_timestamp,
    });

    Ok(())
}

fn validate_existing_job(
    job: &Account<'_, Job>,
    args: &SubmitDataUpdateArgs,
    job_init: Option<&SubmitJobInitArgs>,
) -> Result<()> {
    require!(
        job.job_id == args.job_id,
        MolphaError::DataUpdateJobMismatch
    );
    let expected_job_id = derive_job_id(&job.owner, &job.api_config_hash, job.signatures_required);
    require!(job.job_id == expected_job_id, MolphaError::InvalidJobId);
    require!(
        job.signatures_required == args.signatures_required,
        MolphaError::DataUpdateConfigMismatch
    );
    if let Some(job_init) = job_init {
        require!(
            job.api_config_hash == job_init.api_config_hash,
            MolphaError::DataUpdateConfigMismatch
        );
        require!(
            job.decimals == job_init.decimals,
            MolphaError::DataUpdateConfigMismatch
        );
    }
    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SubmitDataUpdateArgs, job_init: Option<SubmitJobInitArgs>)]
pub struct SubmitDataUpdate<'info> {
    #[account(mut)]
    pub submitter: Signer<'info>,

    /// CHECK: required only when `job` is initialized by this submit.
    pub job_owner: Option<UncheckedAccount<'info>>,

    /// Plan/account validation is performed only when a new job is initialized.
    pub plan: Option<Account<'info, Plan>>,

    #[account(mut)]
    pub subscription: Option<Account<'info, Subscription>>,

    /// Required when a delegate initializes the job on behalf of `job_owner`.
    #[account(mut)]
    pub delegate: Option<Account<'info, Delegate>>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        init_if_needed,
        payer = submitter,
        space = Job::SPACE,
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump
    )]
    pub job: Account<'info, Job>,

    #[account(
        init_if_needed,
        payer = submitter,
        space = Feed::SPACE,
        seeds = [Feed::SEED_PREFIX, args.job_id.as_ref()],
        bump
    )]
    pub feed: Account<'info, Feed>,

    pub system_program: Program<'info, System>,
}

#[cfg(test)]
mod tests {
    use super::SubmitDataUpdateArgs;
    use anchor_lang::{AnchorDeserialize, AnchorSerialize};

    const ARGS_SERIALIZED_LEN: usize = 161;

    fn fixture_args() -> SubmitDataUpdateArgs {
        SubmitDataUpdateArgs {
            job_id: [0x11; 32],
            signatures_required: 3,
            registry_version: 7,
            signers_bitmap: [0x33; 32],
            value: [0x44; 32],
            canonical_timestamp: 1_700_000_000,
            agg_sig_s: [0x66; 32],
            commitment_addr: [0x77; 20],
        }
    }

    #[test]
    fn submit_data_update_args_roundtrip() {
        let args = fixture_args();
        let bytes = args
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        let decoded = SubmitDataUpdateArgs::try_from_slice(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");

        assert_eq!(decoded.job_id, args.job_id);
        assert_eq!(decoded.registry_version, args.registry_version);
        assert_eq!(decoded.signers_bitmap, args.signers_bitmap);
        assert_eq!(decoded.value, args.value);
        assert_eq!(decoded.canonical_timestamp, args.canonical_timestamp);
        assert_eq!(decoded.agg_sig_s, args.agg_sig_s);
        assert_eq!(decoded.commitment_addr, args.commitment_addr);
    }

    #[test]
    fn submit_data_update_args_serialized_len_is_stable() {
        let bytes = fixture_args()
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        assert_eq!(bytes.len(), ARGS_SERIALIZED_LEN);
    }

    #[test]
    fn submit_data_update_args_preserves_negative_timestamp() {
        let mut args = fixture_args();
        args.canonical_timestamp = -123_456_789;
        let bytes = args
            .try_to_vec()
            .expect("SubmitDataUpdateArgs should serialize");
        let decoded = SubmitDataUpdateArgs::try_from_slice(&bytes)
            .expect("SubmitDataUpdateArgs should deserialize");
        assert_eq!(decoded.canonical_timestamp, -123_456_789);
    }
}
