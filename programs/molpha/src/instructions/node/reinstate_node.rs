//! Restore a `Frozen` node after its freeze window (or earlier by protocol authority).

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeReinstated,
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState, RegistryTransition},
};

pub fn reinstate_node(ctx: Context<ReinstateNode>) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let now = Clock::get()?.unix_timestamp;
    let is_authority = ctx.accounts.caller.key() == config.authority;

    require!(
        is_authority || now >= ctx.accounts.node.frozen_until,
        MolphaError::NodeStillFrozen
    );

    let node_key = ctx.accounts.node.key();
    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    let index: u32 = ctx.accounts.registry_state.append_node(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        node_key,
    )?;

    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Active;

    emit!(NodeReinstated {
        node: node_key,
        node_authority: node.authority,
        index,
        registry_version: ctx.accounts.registry_state.current_version,
        reinstated_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ReinstateNode<'info> {
    /// Permissionless after `frozen_until`; authority may reinstate earlier.
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = caller,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        constraint = node.status == NodeStatus::Frozen @ MolphaError::NodeNotFrozen,
        seeds = [Node::SEED_PREFIX, node.authority.as_ref()],
        bump = node.bump,
    )]
    pub node: Box<Account<'info, Node>>,

    pub system_program: Program<'info, System>,
}
