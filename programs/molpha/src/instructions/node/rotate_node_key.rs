//! Replace a node's signing key, publishing a registry version that names the new key.

use anchor_lang::prelude::*;
use molpha_verifier::{verify_key_rotation, KeyRotationError, KeyRotationProof};

use super::register_node::hash_seed;
use crate::{
    error::MolphaError,
    events::NodeKeyRotated,
    state::{
        Node, NodeStatus, ProtocolConfig, Registry, RegistryState, RegistryTransition,
        Secp256k1KeyClaim,
    },
};

/// A rotation takes both the node authority (the transaction signer) and the node's current
/// signing key, which signs the rotation digest inside `proof`. The new key proves possession
/// of its secret over the same digest, so a rotation cannot install a rogue key.
///
/// The new key is reserved for this node by a `Secp256k1KeyClaim`, as at registration, so it is
/// single-use. The old key's claim stays, so a retired key can never be registered or rotated in
/// again.
///
/// The node keeps its registry index. The new version differs from the last only in that
/// member's key, and the last stays verifiable for its grace window with the old key.
pub fn rotate_node_key(ctx: Context<RotateNodeKey>, proof: KeyRotationProof) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.protocol_config;
    let node = &ctx.accounts.node;

    // Every rotation costs a registry version and a new key claim. One per settlement window
    // bounds that, and a compromised authority cannot churn the registry faster than that.
    let cooldown = i64::try_from(config.max_settlement_delay_seconds).unwrap_or(i64::MAX);
    require!(
        node.key_rotated_at == 0 || now >= node.key_rotated_at.saturating_add(cooldown),
        MolphaError::KeyRotationCooldownActive
    );

    let node_key = node.key();
    let old_key = (node.secp256k1_pubkey_x, node.secp256k1_pubkey_y);
    let new_key = verify_key_rotation(
        &ctx.program_id.to_bytes(),
        &node_key.to_bytes(),
        &old_key,
        &proof,
    )
    .map_err(|error| match error {
        KeyRotationError::InvalidPublicKey => error!(MolphaError::PubkeyNotOnCurve),
        KeyRotationError::InvalidProof => error!(MolphaError::InvalidPoP),
        KeyRotationError::InvalidAuthorization => error!(MolphaError::InvalidKeyRotationSignature),
        KeyRotationError::KeyUnchanged => error!(MolphaError::KeyRotationKeyUnchanged),
    })?;

    ctx.accounts
        .secp256k1_key_claim
        .init(node_key, ctx.bumps.secp256k1_key_claim);

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
        node_key,
        new_key,
    )?;

    let node = &mut ctx.accounts.node;
    node.set_key(new_key, now);

    emit!(NodeKeyRotated {
        node: node_key,
        node_authority: node.authority,
        index,
        registry_version: ctx.accounts.registry_state.current_version,
        old_pubkey_x: old_key.0,
        old_pubkey_y: old_key.1,
        new_pubkey_x: new_key.0,
        new_pubkey_y: new_key.1,
        rotated_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(proof: KeyRotationProof)]
pub struct RotateNodeKey<'info> {
    /// The node authority. Pays for the new registry version and the new key's claim.
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

    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Box<Account<'info, Node>>,

    #[account(
        init,
        payer = node_owner,
        space = Secp256k1KeyClaim::SPACE,
        seeds = [
            Secp256k1KeyClaim::SEED_PREFIX,
            &hash_seed(&proof.new_public_key_compressed),
        ],
        bump,
    )]
    pub secp256k1_key_claim: Box<Account<'info, Secp256k1KeyClaim>>,

    pub system_program: Program<'info, System>,
}
