use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::DelegateAdded, state::Delegate};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AddDelegateArgs {
    pub delegate: Pubkey,
    pub max_jobs_created: u32,
    pub max_data_requests: u64,
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

    let delegate_account = &mut ctx.accounts.delegate_account;
    delegate_account.init(
        ctx.accounts.owner.key(),
        args.delegate,
        args.max_jobs_created,
        args.max_data_requests,
        ctx.bumps.delegate_account,
    );

    emit!(DelegateAdded {
        owner: ctx.accounts.owner.key(),
        delegate: args.delegate,
        max_jobs_created: args.max_jobs_created,
        max_data_requests: args.max_data_requests,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: AddDelegateArgs)]
pub struct AddDelegate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

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
