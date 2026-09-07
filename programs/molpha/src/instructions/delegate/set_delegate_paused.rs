use anchor_lang::prelude::*;

use crate::{
    events::DelegateStatusChanged,
    state::{AgentStatus, Delegate},
};

pub fn set_delegate_paused(ctx: Context<SetDelegatePaused>, paused: bool) -> Result<()> {
    let delegate_account = &mut ctx.accounts.delegate_account;

    let old_status = delegate_account.status;
    delegate_account.status = if paused {
        AgentStatus::Paused
    } else {
        AgentStatus::Active
    };

    emit!(DelegateStatusChanged {
        owner: ctx.accounts.owner.key(),
        delegate: delegate_account.delegate,
        old_status: old_status as u8,
        new_status: delegate_account.status as u8,
        updated_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct SetDelegatePaused<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [
            Delegate::SEED_PREFIX,
            owner.key().as_ref(),
            delegate_account.delegate.as_ref(),
        ],
        bump = delegate_account.bump,
    )]
    pub delegate_account: Account<'info, Delegate>,
}
