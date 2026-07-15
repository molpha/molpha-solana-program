use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::DelegateAdded,
    state::{Delegate, Subscription},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddDelegateArgs {
    pub delegate: Pubkey,
    pub max_jobs_created: u32,
    pub max_data_requests: u64,
    pub max_signers: u8,
}

pub fn add_delegate(ctx: Context<AddDelegate>, args: AddDelegateArgs) -> Result<()> {
    require!(
        args.delegate != ctx.accounts.owner.key(),
        MolphaError::DelegateCannotBeOwner
    );
    require!(
        args.max_jobs_created > 0,
        MolphaError::InvalidDelegateLimits
    );
    require!(
        args.max_data_requests > 0,
        MolphaError::InvalidDelegateLimits
    );

    let subscription = &mut ctx.accounts.subscription;

    require!(subscription.is_active(), MolphaError::SubscriptionNotActive);
    require!(
        args.max_signers > 0 && args.max_signers <= subscription.max_signers,
        MolphaError::SignaturesExceedSubscriptionMax
    );
    subscription.try_add_delegate()?;

    let delegate_account = &mut ctx.accounts.delegate_account;
    delegate_account.init(
        ctx.accounts.owner.key(),
        args.delegate,
        args.max_jobs_created,
        args.max_data_requests,
        args.max_signers,
        ctx.bumps.delegate_account,
    );

    emit!(DelegateAdded {
        owner: ctx.accounts.owner.key(),
        delegate: args.delegate,
        max_jobs_created: args.max_jobs_created,
        max_data_requests: args.max_data_requests,
        max_signers: args.max_signers,
        delegate_count: subscription.delegate_count,
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
