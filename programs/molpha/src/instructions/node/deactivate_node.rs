use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeDeactivated,
    state::{Node, NodeStatus, ProtocolConfig, Registry, RegistryState},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct DeactivateNodeArgs {
    pub node_owner: Pubkey,
    pub remove_index: u16,
}

pub fn deactivate_node(ctx: Context<DeactivateNode>, args: DeactivateNodeArgs) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    require!(
        ctx.accounts.authority.key() == args.node_owner
            || ctx.accounts.authority.key() == config.authority,
        MolphaError::UnauthorizedNodeAuthority
    );

    let current = ctx.accounts.current_registry.load()?;
    require_keys_eq!(
        current.node(usize::from(args.remove_index))?,
        ctx.accounts.node.key(),
        MolphaError::InvalidNodeAccount
    );

    let mut next = ctx.accounts.new_registry.load_init()?;
    next.swap_remove(
        &current,
        args.remove_index,
        ctx.accounts.node.key(),
        ctx.bumps.new_registry,
    )?;

    let clock = Clock::get()?;
    let node = &mut ctx.accounts.node;
    node.status = NodeStatus::Deactivated;
    node.withdrawable_slot = clock.slot.saturating_add(config.withdrawal_cooldown_slots);
    ctx.accounts.registry_state.advance_version();

    emit!(NodeDeactivated {
        node: node.key(),
        owner: node.owner,
        index: u32::from(args.remove_index),
        new_registry_version: ctx.accounts.registry_state.next_version,
        deactivated_at: clock.unix_timestamp,
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
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
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
