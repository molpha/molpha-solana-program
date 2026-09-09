use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::DelegateAdded,
    state::{Delegate, Subscription},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddDelegateArgs {
    pub delegate: Pubkey,
    pub max_data_requests: u64,
}

pub fn add_delegate(ctx: Context<AddDelegate>, args: AddDelegateArgs) -> Result<()> {
    require!(
        args.delegate != ctx.accounts.owner.key(),
        MolphaError::DelegateCannotBeOwner
    );
    require!(
        args.max_data_requests > 0,
        MolphaError::InvalidDelegateLimits
    );

    let subscription = &mut ctx.accounts.subscription;
    let delegate_account = &mut ctx.accounts.delegate_account;

    subscription.try_add_delegate()?;
    delegate_account.init(
        ctx.accounts.owner.key(),
        args.delegate,
        args.max_data_requests,
        ctx.bumps.delegate_account,
    );

    emit!(DelegateAdded {
        owner: ctx.accounts.owner.key(),
        delegate: args.delegate,
        delegate_count: subscription.delegate_count,
        added_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: AddDelegateArgs)]
pub struct AddDelegate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        constraint = subscription.is_active() @ MolphaError::SubscriptionNotActive,
        seeds = [Subscription::SEED_PREFIX, owner.key().as_ref()],
        bump = subscription.bump,
    )]
    pub subscription: Account<'info, Subscription>,

    #[account(
        init,
        payer = owner,
        space = Delegate::SPACE,
        seeds = [
            Delegate::SEED_PREFIX,
            owner.key().as_ref(),
            args.delegate.as_ref(),
        ],
        bump
    )]
    pub delegate_account: Account<'info, Delegate>,

    pub system_program: Program<'info, System>,
}
