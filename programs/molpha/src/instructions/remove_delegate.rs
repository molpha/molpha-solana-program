use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::DelegateRemoved, state::Delegate};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RemoveDelegateArgs {
    pub delegate: Pubkey,
}

pub fn remove_delegate(ctx: Context<RemoveDelegate>, args: RemoveDelegateArgs) -> Result<()> {
    let delegate_account = &ctx.accounts.delegate_account;

    require_keys_eq!(
        delegate_account.owner,
        ctx.accounts.owner.key(),
        MolphaError::UnauthorisedJobOwner
    );
    require_keys_eq!(
        delegate_account.delegate,
        args.delegate,
        MolphaError::DelegateNotFound
    );

    emit!(DelegateRemoved {
        owner: ctx.accounts.owner.key(),
        delegate: args.delegate,
        jobs_created: delegate_account.jobs_created,
        data_requests_used: delegate_account.data_requests_used,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RemoveDelegateArgs)]
pub struct RemoveDelegate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        close = owner,
        seeds = [
            Delegate::SEED_PREFIX,
            owner.key().as_ref(),
            args.delegate.as_ref(),
        ],
        bump = delegate_account.bump,
    )]
    pub delegate_account: Account<'info, Delegate>,
}
