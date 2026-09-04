//! Restore a `Frozen` node after its freeze window (or earlier by protocol authority).

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeReinstated,
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState, LIVELINESS_SCORE_MAX},
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
    let current = ctx.accounts.current_registry.load()?;
    let mut next = ctx.accounts.new_registry.load_init()?;
    next.append(&current, node_key, ctx.bumps.new_registry)?;
    ctx.accounts.registry_state.advance_version();

    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Active;
    node.liveliness_score = LIVELINESS_SCORE_MAX;

    emit!(NodeReinstated {
        node: node_key,
        owner: node.owner,
        index: u32::from(current.node_count),
        registry_version: ctx.accounts.registry_state.next_version,
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
        seeds = [Node::SEED_PREFIX, node.owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Box<Account<'info, Node>>,

    pub system_program: Program<'info, System>,
}
