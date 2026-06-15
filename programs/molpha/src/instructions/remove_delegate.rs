use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::DelegateRemoved, state::Job};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RemoveDelegateArgs {
    pub job_id: [u8; 32],
    pub delegate: Pubkey,
}

pub fn remove_delegate(ctx: Context<RemoveDelegate>, args: RemoveDelegateArgs) -> Result<()> {
    let job = &mut ctx.accounts.job;

    require_keys_eq!(
        job.owner,
        ctx.accounts.owner.key(),
        MolphaError::UnauthorisedJobOwner
    );

    let active = job.delegate_count as usize;
    let pos = job.delegates[..active]
        .iter()
        .position(|d| *d == args.delegate)
        .ok_or(MolphaError::DelegateNotFound)?;

    let last = active - 1;
    if pos != last {
        job.delegates[pos] = job.delegates[last];
    }
    job.delegates[last] = Pubkey::default();
    job.delegate_count -= 1;

    emit!(DelegateRemoved {
        job_id: job.job_id,
        delegate: args.delegate,
        delegate_count: job.delegate_count,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RemoveDelegateArgs)]
pub struct RemoveDelegate<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [Job::SEED_PREFIX, args.job_id.as_ref()],
        bump = job.bump,
    )]
    pub job: Account<'info, Job>,
}
