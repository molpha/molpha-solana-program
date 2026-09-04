use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeFrozenForInactivity,
    state::{Node, NodeStatus, OffenceKind, ProtocolConfig, Registry, RegistryState},
    utils::penalize_node,
};

pub fn freeze_unresponsive_node(
    ctx: Context<FreezeUnresponsiveNode>,
    _node_owner: Pubkey,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        ctx.accounts.node.liveliness_score < config.liveliness_freeze_threshold,
        MolphaError::LivelinessAboveThreshold
    );

    let clock = Clock::get()?;
    let node = &mut ctx.accounts.node;

    penalize_node(
        &ctx.accounts.current_registry,
        &mut ctx.accounts.new_registry,
        &mut ctx.accounts.registry_state,
        node,
        OffenceKind::Inactivity,
        ctx.accounts.protocol_config.withdrawal_cooldown_slots,
        ctx.bumps.new_registry,
        &clock,
    )?;

    emit!(NodeFrozenForInactivity {
        node: node.key(),
        owner: node.owner,
        score: node.liveliness_score,
        frozen_until: node.frozen_until,
        new_registry_version: ctx.accounts.registry_state.current_version,
        frozen_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(_node_owner: Pubkey)]
pub struct FreezeUnresponsiveNode<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,

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
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, _node_owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,

    pub system_program: Program<'info, System>,
}
