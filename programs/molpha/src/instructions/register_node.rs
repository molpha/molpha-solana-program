use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::NodeRegistered,
    state::{Node, NodeOperator, ProtocolConfig, RegistryState},
    utils::registry::verify_pop_stub,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterNodeArgs {
    pub new_index: u32,
    pub registration_salt: [u8; 32],
    pub ip: [u8; 4],
    pub port: u16,
    pub pub_key_x: [u8; 32],
    pub pub_key_y: [u8; 32],
    pub pop_sig_r: [u8; 32],
    pub pop_sig_s: [u8; 32],
    pub security_deposit: u64,
}

pub fn register_node(
    ctx: Context<RegisterNode>,
    args: RegisterNodeArgs,
    node_id: [u8; 32],
) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    require_keys_eq!(
        ctx.accounts.admin_authority.key(),
        ctx.accounts.registry_state.authority,
        MolphaError::UnauthorisedAuthority
    );

    let registry = &mut ctx.accounts.registry_state;
    require!(
        now > registry.previous_expires_at,
        MolphaError::TransitionStillInGrace
    );
    let config = &ctx.accounts.protocol_config;
    require!(
        registry.current_node_count < config.max_node_count,
        MolphaError::NodeCapExceeded
    );
    require!(
        args.new_index == registry.current_node_count,
        MolphaError::InvalidNewIndex
    );

    // TODO: full Schnorr PoP verification (MOLPHA_NODE_POP_V1); stub checks scalar shape only.
    let _pop_r = args.pop_sig_r;
    verify_pop_stub(&args.pop_sig_s)?;

    require!(
        args.security_deposit >= config.minimum_node_deposit,
        MolphaError::InsufficientDeposit
    );
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.owner_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.node_owner.to_account_info(),
            },
        ),
        args.security_deposit,
    )?;

    let owner_key = ctx.accounts.node_owner.key();
    ctx.accounts.node_operator.init(
        owner_key,
        args.new_index,
        args.ip,
        args.port,
        args.security_deposit,
        now,
        ctx.bumps.node_operator,
    );

    ctx.accounts.node.init(
        args.new_index,
        owner_key,
        args.pub_key_x,
        args.pub_key_y,
        ctx.bumps.node,
    );

    registry.record_add_transition(now);

    emit!(NodeRegistered {
        node_id,
        new_index: args.new_index,
        owner: ctx.accounts.node_owner.key(),
        ip: args.ip,
        port: args.port,
        current_version: registry.current_version,
        security_deposit: args.security_deposit,
        registered_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RegisterNodeArgs, node_id: [u8; 32])]
pub struct RegisterNode<'info> {
    pub admin_authority: Signer<'info>,

    #[account(mut)]
    pub node_owner: Signer<'info>,

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
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        init,
        payer = node_owner,
        space = NodeOperator::SPACE,
        seeds = [NodeOperator::SEED_PREFIX, node_owner.key().as_ref()],
        bump
    )]
    pub node_operator: Box<Account<'info, NodeOperator>>,

    #[account(
        init,
        payer = node_owner,
        space = Node::SPACE,
        seeds = [
            Node::SEED_PREFIX,
            args.new_index.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub node: Box<Account<'info, Node>>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Box<Account<'info, token::Mint>>,

    #[account(
        mut,
        constraint = owner_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = owner_usdc.owner == node_owner.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub owner_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
