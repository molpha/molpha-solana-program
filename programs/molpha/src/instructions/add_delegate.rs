use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::DelegateAdded, state::Job};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddDelegateArgs {
    pub job_id: [u8; 32],
    pub delegate: Pubkey,
}

pub fn add_delegate(ctx: Context<AddDelegate>, args: AddDelegateArgs) -> Result<()> {
    let job = &mut ctx.accounts.job;

    require_keys_eq!(
        job.owner,
        ctx.accounts.owner.key(),
        MolphaError::UnauthorisedJobOwner
    );
    require!(
        job.delegate_count < Job::MAX_DELEGATES as u8,
        MolphaError::DelegateLimitReached
    );
    require!(
        !job.has_delegate(&args.delegate),
        MolphaError::DelegateAlreadyPresent
    );

    let idx = job.delegate_count as usize;
    job.delegates[idx] = args.delegate;
    job.delegate_count += 1;

    emit!(DelegateAdded {
        job_id: job.job_id,
        delegate: args.delegate,
        delegate_count: job.delegate_count,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: AddDelegateArgs)]
pub struct AddDelegate<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump = job.bump,
    )]
    pub job: Account<'info, Job>,
}
