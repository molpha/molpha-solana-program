use anchor_lang::prelude::*;

use crate::state::{
    RegistryIndex, RegistryState, RegistryTransitionAccount, RegistryTransitionType,
    VIRTUAL_INDEX,
};

pub fn finalize_registry_transition(ctx: Context<FinalizeRegistryTransition>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let registry = &mut ctx.accounts.registry_state;
    require!(
        now > registry.previous_expires_at,
        crate::error::MolphaError::TransitionStillInGrace
    );

    if let Some(transition) = &mut ctx.accounts.transition_account {
        transition.close(ctx.accounts.rent_receiver.to_account_info())?;
    }

    if let Some(virtual_registry_index) = &mut ctx.accounts.virtual_registry_index {
        virtual_registry_index.close(ctx.accounts.rent_receiver.to_account_info())?;
    }

    registry.previous_version = registry.current_version;
    registry.previous_node_count = registry.current_node_count;
    registry.previous_expires_at = 0;
    registry.last_transition_type = RegistryTransitionType::None;
    registry.transition_account = Pubkey::default();

    Ok(())
}

#[derive(Accounts)]
pub struct FinalizeRegistryTransition<'info> {
    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(mut)]
    pub transition_account: Option<Account<'info, RegistryTransitionAccount>>,

    #[account(
        mut,
        seeds = [
            RegistryIndex::SEED_PREFIX,
            VIRTUAL_INDEX.to_le_bytes().as_ref(),
        ],
        bump = virtual_registry_index.bump,
    )]
    pub virtual_registry_index: Option<Account<'info, RegistryIndex>>,

    /// CHECK: receives rent from transition close.
    #[account(mut)]
    pub rent_receiver: UncheckedAccount<'info>,
}
