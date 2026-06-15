use anchor_lang::prelude::*;

use crate::{
    instructions::{
        data_update_core::build_coalition_and_verify, submit_data_update::SubmitDataUpdateArgs,
    },
    state::RegistryState,
};

/// Return-data layout (72 bytes):
/// - `[0..32]`  `value [u8; 32]`
/// - `[32..40]` `canonical_timestamp i64` big-endian
const RETURN_LEN: usize = 72;

pub fn verify_data_update(
    ctx: Context<VerifyDataUpdate>,
    args: SubmitDataUpdateArgs,
) -> Result<()> {
    let registry = &ctx.accounts.registry_state;

    let clock = Clock::get()?;
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
}
