use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    instructions::{
        data_update_core::build_coalition_and_verify, submit_data_update::SubmitDataUpdateArgs,
    },
    state::{Job, RegistryState},
};

/// Return-data layout (72 bytes):
/// - `[0..32]`  `value [u8; 32]`
/// - `[32..40]` `canonical_timestamp i64` big-endian
const RETURN_LEN: usize = 72;

pub fn verify_data_update(
    ctx: Context<VerifyDataUpdate>,
    args: SubmitDataUpdateArgs,
) -> Result<()> {
    let job = &ctx.accounts.job;
    let registry = &ctx.accounts.registry_state;

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    build_coalition_and_verify(
        &args,
        &job.api_config_hash,
        registry,
        now,
        ctx.remaining_accounts,
        ctx.program_id,
        args.signatures_required,
        registry.redundancy_buffer,
    )?;

    let mut return_data = [0u8; RETURN_LEN];
    return_data[0..32].copy_from_slice(&args.value);
    return_data[32..40].copy_from_slice(&args.canonical_timestamp.to_be_bytes());
    anchor_lang::solana_program::program::set_return_data(&return_data);

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SubmitDataUpdateArgs)]
pub struct VerifyDataUpdate<'info> {
    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump = job.bump,
        constraint = job.job_id == args.job_id @ MolphaError::DataUpdateJobMismatch,
    )]
    pub job: Account<'info, Job>,
}
