use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::NodeRegistered,
    state::{
        Node, NodeStatus, ProtocolConfig, RegistryIndex, RegistryState,
        RegistryTransitionType,
    },
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
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.owner_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.node_owner.to_account_info(),
            },
        ),
        args.security_deposit,
    )?;

    let node = &mut ctx.accounts.node;
    node.owner = ctx.accounts.node_owner.key();
    node.current_index = args.new_index;
    node.status = NodeStatus::Active;
    node.locked_amount = args.security_deposit;
    node.pending_rewards = 0;
    node.registered_at = now;
    node.deactivated_at = 0;
    node.withdrawable_at = 0;
    node.bump = ctx.bumps.node;

    let index = &mut ctx.accounts.registry_index;
    index.index = args.new_index;
    index.node = ctx.accounts.node.key();
    index.secp256k1_pubkey_x = args.pub_key_x;
    index.secp256k1_pubkey_y = args.pub_key_y;
    index.bump = ctx.bumps.registry_index;

    let current_version = registry.current_version;
    registry.previous_version = current_version;
    registry.previous_node_count = registry.current_node_count;
    registry.previous_expires_at = now.saturating_add(registry.grace_secs);
    registry.current_version = current_version.saturating_add(1);
    registry.current_node_count = registry.current_node_count.saturating_add(1);
    registry.last_transition_type = RegistryTransitionType::Add;
    registry.clear_remove_transition_metadata();

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
        space = Node::SPACE,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump
    )]
    pub node: Box<Account<'info, Node>>,

    #[account(
        init,
        payer = node_owner,
        space = RegistryIndex::SPACE,
        seeds = [
            RegistryIndex::SEED_PREFIX,
            args.new_index.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub registry_index: Box<Account<'info, RegistryIndex>>,

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
