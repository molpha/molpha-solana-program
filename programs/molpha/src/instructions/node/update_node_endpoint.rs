//! A member moves where the gateway reaches it: a node its public address, a provider its libp2p
//! identity. No registry version: the signing key, which verification uses, does not change.

use anchor_lang::prelude::*;

use super::register_node::MemberEndpoint;
use crate::{
    error::MolphaError,
    events::{NodeEndpointUpdated, ProviderP2pKeyUpdated},
    state::{Node, NodeStatus, Provider, ProviderStatus},
    utils::require_valid_p2p_key,
};

/// Pass exactly one of `node` and `provider`; `endpoint`'s variant must match it, as at
/// registration.
pub fn update_node_endpoint(
    ctx: Context<UpdateNodeEndpoint>,
    endpoint: MemberEndpoint,
) -> Result<()> {
    let accounts = ctx.accounts;
    match (endpoint, &mut accounts.node, &mut accounts.provider) {
        (MemberEndpoint::Node { ip, port }, Some(node), None) => {
            require!(
                ip != [0, 0, 0, 0] && port != 0,
                MolphaError::InvalidNodeEndpoint
            );
            node.update_endpoint(ip, port);
            emit!(NodeEndpointUpdated {
                node: node.key(),
                node_authority: node.authority,
                ip,
                port,
                updated_at: Clock::get()?.unix_timestamp,
            });
        }
        (MemberEndpoint::Provider { p2p_key }, None, Some(provider)) => {
            require_valid_p2p_key(&p2p_key)?;
            let old_p2p_key = provider.p2p_key;
            provider.p2p_key = p2p_key;
            emit!(ProviderP2pKeyUpdated {
                provider: provider.key(),
                id: provider.id,
                old_p2p_key,
                new_p2p_key: p2p_key,
            });
        }
        (_, Some(_), None) | (_, None, Some(_)) => return err!(MolphaError::MemberKindMismatch),
        _ => return err!(MolphaError::ExactlyOneMember),
    }
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateNodeEndpoint<'info> {
    /// The node owner, or the provider authority.
    pub node_owner: Signer<'info>,

    /// The node that moves; omit it for a provider.
    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Option<Box<Account<'info, Node>>>,

    /// The provider whose libp2p identity changes; omit it for a node.
    #[account(
        mut,
        constraint = provider.authority == node_owner.key() @ MolphaError::UnauthorizedNodeAuthority,
        constraint = provider.status == ProviderStatus::Active @ MolphaError::ProviderRemoved,
        seeds = [Provider::SEED_PREFIX, provider.id.to_le_bytes().as_ref()],
        bump = provider.bump,
    )]
    pub provider: Option<Box<Account<'info, Provider>>>,
}
