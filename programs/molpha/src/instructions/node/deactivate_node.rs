use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::{NodeDeactivated, ProviderEjected, RegistryUpdated, RegistryVersionReason},
    state::{
        Node, NodeStatus, ProtocolConfig, Provider, ProviderStatus, Registry, RegistryRef,
        RegistryState, RegistryTransition,
    },
};

/// A node or a provider leaves the registry by its own decision; a node can also leave by
/// evidence (`slash_offence`). The protocol authority cannot remove anyone.
///
/// Pass exactly one of `node` and `provider`. For a node, `node_owner` is its owner and must be
/// the signer. For a provider, `node_owner` is unused (any key) and the signer must be the
/// provider authority.
///
/// `Removed` is terminal for a provider: its id is not reused.
pub fn deactivate_node(ctx: Context<DeactivateNode>, node_owner: Pubkey) -> Result<()> {
    match (&ctx.accounts.node, &ctx.accounts.provider) {
        (Some(_), None) => deactivate_node_member(ctx, node_owner),
        (None, Some(_)) => eject_provider_member(ctx),
        _ => err!(MolphaError::ExactlyOneMember),
    }
}

fn deactivate_node_member(ctx: Context<DeactivateNode>, node_owner: Pubkey) -> Result<()> {
    require!(
        ctx.accounts.authority.key() == node_owner,
        MolphaError::UnauthorizedNodeAuthority
    );
    let node = ctx.accounts.node.as_ref().unwrap();
    require!(
        node.status == NodeStatus::Active,
        MolphaError::NodeNotActive
    );
    let node_key = node.key();

    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    ctx.accounts.registry_state.remove_node(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            payer: &ctx.accounts.authority,
            system_program: &ctx.accounts.system_program,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        node_key,
    )?;

    let cooldown = ctx.accounts.protocol_config.withdrawal_cooldown_slots;
    let clock = Clock::get()?;
    let node = ctx.accounts.node.as_mut().unwrap();
    node.status = NodeStatus::Deactivated;
    node.withdrawable_slot = clock.slot.saturating_add(cooldown);

    emit!(NodeDeactivated {
        node: node.key(),
        node_authority: node.authority,
        new_registry_version: ctx.accounts.registry_state.current_version,
        deactivated_at: clock.unix_timestamp,
    });

    Ok(())
}

fn eject_provider_member(ctx: Context<DeactivateNode>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let provider = ctx.accounts.provider.as_ref().unwrap();
    let provider_key = provider.key();
    require!(
        provider.status == ProviderStatus::Active,
        MolphaError::ProviderRemoved
    );

    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    ctx.accounts.registry_state.remove_provider(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            payer: &ctx.accounts.authority,
            system_program: &ctx.accounts.system_program,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        provider_key,
    )?;
    let registry = RegistryRef::load(&ctx.accounts.new_registry)?;
    emit!(RegistryUpdated {
        new_version: ctx.accounts.registry_state.current_version,
        node_count: u16::from(registry.node_count),
        redundancy_buffer: registry.redundancy_buffer,
        reason: RegistryVersionReason::ProviderEjected,
        updated_at: now,
        provider_count: u16::from(registry.provider_count),
    });

    let provider = ctx.accounts.provider.as_mut().unwrap();
    provider.status = ProviderStatus::Removed;
    emit!(ProviderEjected {
        provider: provider_key,
        id: provider.id,
        authority: provider.authority,
        slot: provider.slot,
        registry_version: ctx.accounts.registry_state.current_version,
        ejected_at: now,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(node_owner: Pubkey)]
pub struct DeactivateNode<'info> {
    /// The node owner, or the provider authority. Pays for the new registry version.
    #[account(mut)]
    pub authority: Signer<'info>,

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

    /// CHECK: the next registry version's PDA. The handler creates it, or takes over one that
    /// `extend_registry` allocated, and writes the snapshot.
    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: UncheckedAccount<'info>,

    /// The node to deactivate; omit it when a provider leaves.
    #[account(
        mut,
        seeds = [Node::SEED_PREFIX, node_owner.as_ref()],
        bump = node.bump,
    )]
    pub node: Option<Box<Account<'info, Node>>>,

    /// The provider to eject; omit it when a node leaves.
    #[account(
        mut,
        has_one = authority @ MolphaError::UnauthorizedNodeAuthority,
        seeds = [Provider::SEED_PREFIX, provider.id.to_le_bytes().as_ref()],
        bump = provider.bump,
    )]
    pub provider: Option<Box<Account<'info, Provider>>>,

    pub system_program: Program<'info, System>,
}
