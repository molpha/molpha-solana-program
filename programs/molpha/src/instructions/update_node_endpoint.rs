use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::NodeEndpointUpdated,
    state::{NodeOperator, NodeStatus},
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

    let node_operator = &mut ctx.accounts.node_operator;
    require!(
        node_operator.status == NodeStatus::Active,
        MolphaError::NodeNotActive
    );

    node_operator.update_endpoint(args.ip, args.port);

    emit!(NodeEndpointUpdated {
        owner: node_operator.owner,
        current_index: node_operator.current_index,
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
        seeds = [NodeOperator::SEED_PREFIX, node_owner.key().as_ref()],
        bump = node_operator.bump,
        constraint = node_operator.owner == node_owner.key() @ MolphaError::UnauthorizedNodeAuthority,
    )]
    pub node_operator: Account<'info, NodeOperator>,
}
