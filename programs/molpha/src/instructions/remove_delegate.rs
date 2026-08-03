use anchor_lang::prelude::*;

use crate::{
    events::DelegateRemoved,
    state::{Delegate, Subscription},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RemoveDelegateArgs {
    pub delegate: Pubkey,
}

pub fn remove_delegate(ctx: Context<RemoveDelegate>, args: RemoveDelegateArgs) -> Result<()> {
    let delegate_account = &ctx.accounts.delegate_account;

    let subscription = &mut ctx.accounts.subscription;
    subscription.release_delegate();

    emit!(DelegateRemoved {
        owner: ctx.accounts.owner.key(),
        delegate: args.delegate,
        data_requests_used: delegate_account.data_requests_used,
        delegate_count: subscription.delegate_count,
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
        seeds = [Subscription::SEED_PREFIX, owner.key().as_ref()],
        bump = subscription.bump,
    )]
    pub subscription: Account<'info, Subscription>,

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
