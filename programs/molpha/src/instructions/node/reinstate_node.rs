//! Restore a `Frozen` node after its freeze window (or earlier by protocol authority).

use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{NodeReinstated, RegistryVersionCreated, RegistryVersionReason},
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ReinstateNodeArgs {
    pub node_owner: Pubkey,
    pub new_registry_version: u32,
}

pub fn reinstate_node(ctx: Context<ReinstateNode>, args: ReinstateNodeArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.node.owner,
        args.node_owner,
        MolphaError::InvalidNodeAccount
    );
    require!(
        ctx.accounts.node.status != NodeStatus::Tombstoned,
        MolphaError::NodeTombstoned
    );
    require!(
        ctx.accounts.node.status == NodeStatus::Frozen,
        MolphaError::NodeNotFrozen
    );

    let config = &ctx.accounts.protocol_config;
    let now = Clock::get()?.unix_timestamp;
    let is_authority = ctx.accounts.caller.key() == config.authority;
    require!(
        is_authority || now >= ctx.accounts.node.frozen_until,
        MolphaError::NodeStillFrozen
    );

    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

    let previous_version = ctx.accounts.registry_state.current_version;
    let node_key = ctx.accounts.node.key();
    let max_node_count = config.max_node_count;
    let (node_count, redundancy_buffer) = {
        let current = ctx.accounts.current_registry.load()?;
        require!(
            !(0..usize::from(current.node_count))
                .any(|i| Pubkey::new_from_array(current.nodes[i]) == node_key),
            MolphaError::NodeAlreadyInRegistry
        );
        let mut next = ctx.accounts.new_registry.load_init()?;
        next.copy_from_previous(&current, args.new_registry_version, ctx.bumps.new_registry);
        next.append(node_key, max_node_count)?;
        (next.node_count, next.redundancy_buffer)
    };
    ctx.accounts.registry_state.current_version = args.new_registry_version;

    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Active;

    emit!(NodeReinstated {
        node: node_key,
        owner: node.owner,
        registry_version: args.new_registry_version,
        reinstated_at: now,
    });
    emit!(RegistryVersionCreated {
        previous_version,
        new_version: args.new_registry_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::NodeReinstated,
        created_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: ReinstateNodeArgs)]
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
        seeds = [Registry::SEED_PREFIX, args.new_registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Node::SEED_PREFIX, args.node_owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Box<Account<'info, Node>>,

    pub system_program: Program<'info, System>,
}
