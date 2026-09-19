use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeDeactivated,
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState, RegistryTransition},
};

pub fn deactivate_node(ctx: Context<DeactivateNode>, node_owner: Pubkey) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        ctx.accounts.authority.key() == node_owner
            || ctx.accounts.authority.key() == config.authority,
        MolphaError::UnauthorizedNodeAuthority
    );

    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    ctx.accounts.registry_state.swap_remove_node(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        ctx.accounts.node.key(),
    )?;

    let clock = Clock::get()?;
    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Deactivated;
    node.withdrawable_slot = clock.slot.saturating_add(config.withdrawal_cooldown_slots);

    emit!(NodeDeactivated {
        node: node.key(),
        node_authority: node.authority,
        new_registry_version: ctx.accounts.registry_state.next_version,
        deactivated_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(node_owner: Pubkey)]
pub struct DeactivateNode<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = authority,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, node_owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,

    pub system_program: Program<'info, System>,
}
