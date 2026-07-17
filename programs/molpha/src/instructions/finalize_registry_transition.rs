use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    state::{Node, NodeOperator, RegistryState, RegistryTransitionType, VIRTUAL_INDEX},
};

pub fn finalize_registry_transition(ctx: Context<FinalizeRegistryTransition>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let registry = &mut ctx.accounts.registry_state;
    require!(
        now > registry.previous_expires_at,
        MolphaError::TransitionStillInGrace
    );

    if let Some(virtual_node) = &mut ctx.accounts.virtual_node {
        // Migrate any rewards left on the virtual slot to the deactivated operator's claimable
        // bucket so it can still be withdrawn, then close the virtual slot.
        let operator = ctx
            .accounts
            .node_operator
            .as_mut()
            .ok_or(error!(MolphaError::InvalidNodeAccount))?;
        require!(
            operator.owner == virtual_node.owner,
            MolphaError::InvalidNodeAccount
        );
        operator.claimable_rewards = operator
            .claimable_rewards
            .checked_add(virtual_node.claimable_rewards)
            .ok_or(error!(MolphaError::InvalidNodeAccount))?;

        virtual_node.close(ctx.accounts.rent_receiver.to_account_info())?;
    }

    registry.previous_version = registry.current_version;
    registry.previous_node_count = registry.current_node_count;
    registry.previous_expires_at = 0;
    registry.last_transition_type = RegistryTransitionType::None;
    registry.clear_remove_transition_metadata();

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

    #[account(
        mut,
        seeds = [
            Node::SEED_PREFIX,
            VIRTUAL_INDEX.to_le_bytes().as_ref(),
        ],
        bump = virtual_node.bump,
    )]
    pub virtual_node: Option<Account<'info, Node>>,

    /// The deactivated operator that receives the virtual slot's residual rewards. Required
    /// whenever `virtual_node` is present.
    #[account(mut)]
    pub node_operator: Option<Account<'info, NodeOperator>>,

    /// CHECK: receives rent from transition close.
    #[account(mut)]
    pub rent_receiver: UncheckedAccount<'info>,
}
