use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use solana_keccak_hasher::hashv;

use crate::{
    error::MolphaError,
    events::{NodeRegistered, RegistryVersionCreated, RegistryVersionReason},
    state::{Node, ProtocolConfig, Registry, RegistryState, Secp256k1KeyClaim},
    utils::registry::{
        derive_node_id, validate_secp256k1_pubkey, verify_node_pop,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterNodeArgs {
    pub new_registry_version: u32,
    pub registration_salt: [u8; 32],
    pub ip: [u8; 4],
    pub port: u16,
    pub secp256k1_pubkey_compressed: [u8; 33],
    pub pop_sig_r: [u8; 32],
    pub pop_sig_s: [u8; 32],
}

pub fn register_node(ctx: Context<RegisterNode>, args: RegisterNodeArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidNodeEndpoint
    );
    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

    let node_index = {
        let registry = ctx.accounts.current_registry.load()?;
        require!(
            u32::from(registry.node_count) < ctx.accounts.protocol_config.max_node_count,
            MolphaError::NodeCapExceeded
        );
        registry.node_count
    };

    require!(
        args.registration_salt != [0u8; 32],
        MolphaError::InvalidRegistrationSalt
    );
    let (pub_key_x, pub_key_y) = validate_secp256k1_pubkey(&args.secp256k1_pubkey_compressed)?;
    let owner = ctx.accounts.node_owner.key();
    let node_id = derive_node_id(
        owner,
        &args.secp256k1_pubkey_compressed,
        &args.registration_salt,
    );
    verify_node_pop(
        ctx.program_id,
        &node_id,
        &args.secp256k1_pubkey_compressed,
        &args.pop_sig_r,
        &args.pop_sig_s,
    )?;

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.owner_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.node_owner.to_account_info(),
            },
        ),
        ctx.accounts.protocol_config.minimum_node_deposit,
    )?;

    let now = Clock::get()?.unix_timestamp;
    ctx.accounts.node.init(
        owner,
        pub_key_x,
        pub_key_y,
        args.ip,
        args.port,
        ctx.accounts.protocol_config.minimum_node_deposit,
        now,
        ctx.bumps.node,
    );
    ctx.accounts.secp256k1_key_claim.init(
        ctx.accounts.node.key(),
        ctx.bumps.secp256k1_key_claim,
    );

    let (node_count, redundancy_buffer) = {
        let current = ctx.accounts.current_registry.load()?;
        let mut next = ctx.accounts.new_registry.load_init()?;
        next.copy_from_previous(&current, args.new_registry_version, ctx.bumps.new_registry);
        let appended_index = next.append(
            ctx.accounts.node.key(),
            ctx.accounts.protocol_config.max_node_count,
        )?;
        require!(appended_index == node_index, MolphaError::InvalidNodeIndex);
        (next.node_count, next.redundancy_buffer)
    };
    let current_version = ctx.accounts.registry_state.current_version;
    ctx.accounts.registry_state.current_version = args.new_registry_version;

    emit!(NodeRegistered {
        node_id,
        new_index: u32::from(node_index),
        owner,
        ip: args.ip,
        port: args.port,
        current_version: args.new_registry_version,
        security_deposit: ctx.accounts.protocol_config.minimum_node_deposit,
        registered_at: now,
    });
    emit!(RegistryVersionCreated {
        previous_version: current_version,
        new_version: args.new_registry_version,
        node_count,
        redundancy_buffer,
        reason: RegistryVersionReason::NodeAdded,
        created_at: now,
    });
    Ok(())
}

pub fn hash_seed(input: &[u8]) -> [u8; 32] {
    let hash_result = hashv(&[input.as_ref()]);
    hash_result.to_bytes()
}

#[derive(Accounts)]
#[instruction(args: RegisterNodeArgs)]
pub struct RegisterNode<'info> {
    pub authority: Signer<'info>,

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
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = node_owner,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, args.new_registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = node_owner,
        space = Node::SPACE,
        seeds = [
            Node::SEED_PREFIX, 
            node_owner.key().as_ref()
        ],
        bump,
    )]
    pub node: Box<Account<'info, Node>>,

    #[account(
        init,
        payer = node_owner,
        space = Secp256k1KeyClaim::SPACE,
        seeds = [
            Secp256k1KeyClaim::SEED_PREFIX,
            &hash_seed(&args.secp256k1_pubkey_compressed),
        ],
        bump,
    )]
    pub secp256k1_key_claim: Box<Account<'info, Secp256k1KeyClaim>>,

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
