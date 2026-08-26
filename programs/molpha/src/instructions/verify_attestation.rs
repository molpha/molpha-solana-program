use anchor_lang::prelude::*;
use molpha_verifier::{solana::verify_attestation_accounts, Attestation};

use crate::{
    error::map_attestation_accounts_err,
    state::Registry,
};

use super::SubmitAttestationArgs;

/// Verify an attestation against any retained historical registry without mutating live state.
pub fn verify_attestation(
    ctx: Context<VerifyAttestation>,
    args: SubmitAttestationArgs,
) -> Result<()> {
    let attestation: Attestation = (&args).into();
    verify_attestation_accounts(
        &attestation,
        &ctx.accounts.registry.to_account_info(),
        &ctx.remaining_accounts,
        &ctx.program_id,
    )
    .map_err(map_attestation_accounts_err)
}

#[derive(Accounts)]
#[instruction(args: SubmitAttestationArgs)]
pub struct VerifyAttestation<'info> {
    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,
}
