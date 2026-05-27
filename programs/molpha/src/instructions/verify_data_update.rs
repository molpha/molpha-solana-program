use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    instructions::{
        data_update_core::build_coalition_and_verify, submit_data_update::SubmitDataUpdateArgs,
    },
    state::{Job, ProtocolConfig, RegistryState, RegistryTransitionAccount},
};

/// Return-data layout (72 bytes):
/// - `[0..32]`  `value [u8; 32]`
/// - `[32..40]` `canonical_timestamp i64` big-endian
/// - `[40..72]` `round_id [u8; 32]`
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
        registry,
        &ctx.accounts.protocol_config,
        job,
        ctx.accounts.transition_account.as_ref().map(|a| a.as_ref()),
        now,
        ctx.remaining_accounts,
        ctx.program_id,
    )?;

    let mut return_data = [0u8; RETURN_LEN];
    return_data[0..32].copy_from_slice(&args.value);
    return_data[32..40].copy_from_slice(&args.canonical_timestamp.to_be_bytes());
    return_data[40..72].copy_from_slice(&args.round_id);
    anchor_lang::solana_program::program::set_return_data(&return_data);

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SubmitDataUpdateArgs)]
pub struct VerifyDataUpdate<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    pub transition_account: Option<Account<'info, RegistryTransitionAccount>>,

    #[account(
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump = job.bump,
        constraint = job.job_id == args.job_id @ MolphaError::DataUpdateJobMismatch,
    )]
    pub job: Account<'info, Job>,
}
