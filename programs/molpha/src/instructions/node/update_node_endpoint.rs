use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeEndpointUpdated,
    state::{Node, NodeStatus},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UpdateNodeEndpointArgs {
    pub ip: [u8; 4],
    pub port: u16,
}

pub fn update_node_endpoint(
    ctx: Context<UpdateNodeEndpoint>,
    args: UpdateNodeEndpointArgs,
) -> Result<()> {
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidNodeEndpoint
    );

    let node = &mut ctx.accounts.node;
    node.update_endpoint(args.ip, args.port);

    emit!(NodeEndpointUpdated {
        node: node.key(),
        owner: node.owner,
        ip: args.ip,
        port: args.port,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateNodeEndpoint<'info> {
    pub node_owner: Signer<'info>,

    #[account(
        mut,
        constraint = node.status == NodeStatus::Active @ MolphaError::NodeNotActive,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump = node.bump,
    )]
    pub node: Account<'info, Node>,
}
