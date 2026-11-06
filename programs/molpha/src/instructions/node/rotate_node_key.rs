//! Replace a node's or a provider's signing key, publishing a registry version that names the
//! new key.

use anchor_lang::prelude::*;
use molpha_verifier::{verify_key_rotation, KeyRotationProof};

use crate::{
    error::MolphaError,
    events::{NodeKeyRotated, ProviderKeyRotated},
    state::{
        compressed_key_x, KeyXClaim, Node, NodeStatus, ProtocolConfig, Provider, ProviderStatus,
        Registry, RegistryState, RegistryTransition,
    },
    utils::rotation_error,
};

/// A rotation takes both the authority (the transaction signer) and the member's current
/// signing key, which signs the rotation digest inside `proof`. The new key proves possession
/// of its secret over the same digest, so a rotation cannot install a rogue key.
///
/// The member is either a node (`node`) or a provider (`provider`): pass exactly one. The proof
/// is bound to the member's account address, so it cannot be replayed on another member, and a
/// node and a provider never share an address.
///
/// The new key's x coordinate is reserved for this member by a `KeyXClaim`, as at registration, so
/// the key (and its negation) is single-use. The old key's claim stays, so a retired key can never be registered or rotated in
/// again.
///
/// The member keeps its registry index. The new version differs from the last only in that
/// member's key, and the last stays verifiable for its grace window with the old key.
pub fn rotate_node_key(ctx: Context<RotateNodeKey>, proof: KeyRotationProof) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let (member_key, old_key, key_rotated_at) = match (&ctx.accounts.node, &ctx.accounts.provider) {
        (Some(node), None) => (
            node.key(),
            (node.secp256k1_pubkey_x, node.secp256k1_pubkey_y),
            node.key_rotated_at,
        ),
        (None, Some(provider)) => (provider.key(), provider.xy(), provider.key_rotated_at),
        _ => return err!(MolphaError::ExactlyOneMember),
    };

    // Every rotation costs a registry version and a new key claim. One per settlement window
    // bounds that, and a compromised authority cannot churn the registry faster than that.
    let cooldown = i64::try_from(ctx.accounts.protocol_config.max_settlement_delay_seconds)
        .unwrap_or(i64::MAX);
    require!(
        key_rotated_at == 0 || now >= key_rotated_at.saturating_add(cooldown),
        MolphaError::KeyRotationCooldownActive
    );

    let new_key = verify_key_rotation(
        &ctx.program_id.to_bytes(),
        &member_key.to_bytes(),
        &old_key,
        &proof,
    )
    .map_err(rotation_error)?;

    ctx.accounts
        .key_x_claim
        .init(member_key, ctx.bumps.key_x_claim);

    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    let index = ctx.accounts.registry_state.replace_node_key(
        &RegistryTransition {
            current: &ctx.accounts.current_registry,
            next: &ctx.accounts.new_registry,
            payer: &ctx.accounts.node_owner,
            system_program: &ctx.accounts.system_program,
            grace_active_until,
            bump: ctx.bumps.new_registry,
        },
        member_key,
        new_key,
    )?;
    let registry_version = ctx.accounts.registry_state.current_version;

    if let Some(node) = ctx.accounts.node.as_mut() {
        node.set_key(new_key, now);
        emit!(NodeKeyRotated {
            node: member_key,
            node_authority: node.authority,
            index,
            registry_version,
            old_pubkey_x: old_key.0,
            old_pubkey_y: old_key.1,
            new_pubkey_x: new_key.0,
            new_pubkey_y: new_key.1,
            rotated_at: now,
        });
    } else if let Some(provider) = ctx.accounts.provider.as_mut() {
        provider.set_key(new_key, now);
        emit!(ProviderKeyRotated {
            provider: member_key,
            id: provider.id,
            authority: provider.authority,
            slot: provider.slot,
            registry_version,
            old_pubkey_x: old_key.0,
            old_pubkey_y: old_key.1,
            new_pubkey_x: new_key.0,
            new_pubkey_y: new_key.1,
            rotated_at: now,
        });
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction(proof: KeyRotationProof)]
pub struct RotateNodeKey<'info> {
    /// The node authority, or the provider authority. Pays for the new registry version and the
    /// new key's claim.
    #[account(mut)]
    pub node_owner: Signer<'info>,

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

    /// The node whose key rotates; omit it when a provider rotates.
    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Option<Box<Account<'info, Node>>>,

    /// The provider whose key rotates; omit it when a node rotates. It must be in the registry.
    #[account(
        mut,
        constraint = provider.authority == node_owner.key() @ MolphaError::UnauthorizedNodeAuthority,
        constraint = provider.status == ProviderStatus::Active @ MolphaError::ProviderNotActive,
        seeds = [Provider::SEED_PREFIX, provider.id.to_le_bytes().as_ref()],
        bump = provider.bump,
    )]
    pub provider: Option<Box<Account<'info, Provider>>>,

    #[account(
        init,
        payer = node_owner,
        space = KeyXClaim::SPACE,
        seeds = [KeyXClaim::SEED_PREFIX, compressed_key_x(&proof.new_public_key_compressed)],
        bump,
    )]
    pub key_x_claim: Box<Account<'info, KeyXClaim>>,

    pub system_program: Program<'info, System>,
}
