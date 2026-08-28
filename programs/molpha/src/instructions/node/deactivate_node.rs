use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{NodeDeactivated, RegistryVersionCreated, RegistryVersionReason},
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct DeactivateNodeArgs {
    pub node_owner: Pubkey,
    pub remove_index: u16,
    pub new_registry_version: u32,
}

pub fn deactivate_node(ctx: Context<DeactivateNode>, args: DeactivateNodeArgs) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        ctx.accounts.authority.key() == args.node_owner
            || ctx.accounts.authority.key() == config.authority,
        MolphaError::UnauthorizedNodeAuthority
    );

    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

    let previous_version = ctx.accounts.registry_state.current_version;
    let (node_count, redundancy_buffer) = {
        let current = ctx.accounts.current_registry.load()?;
        require_keys_eq!(
            current.node(usize::from(args.remove_index))?,
            ctx.accounts.node.key(),
            MolphaError::InvalidNodeAccount
        );

        let mut next = ctx.accounts.new_registry.load_init()?;
        next.copy_from_previous(&current, args.new_registry_version, ctx.bumps.new_registry);
        next.swap_remove(args.remove_index, ctx.accounts.node.key())?;
        (next.node_count, next.redundancy_buffer)
    };

    let now = Clock::get()?.unix_timestamp;
    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Deactivated;
    node.deactivated_at = now;
    node.withdrawable_at = now.saturating_add(config.withdrawal_cooldown_secs);
    ctx.accounts.registry_state.current_version = args.new_registry_version;

    emit!(NodeDeactivated {
        node: node.key(),
        owner: node.owner,
        removed_index: u32::from(args.remove_index),
        current_version: args.new_registry_version,
        deactivated_at: now,
    });
    emit!(RegistryVersionCreated {
        previous_version,
        new_version: args.new_registry_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::NodeRemoved,
        created_at: now,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(args: DeactivateNodeArgs)]
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
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = authority,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, args.new_registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, args.node_owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,

    pub system_program: Program<'info, System>,
}
