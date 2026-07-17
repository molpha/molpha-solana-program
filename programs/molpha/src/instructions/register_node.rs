use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::NodeRegistered,
    state::{Node, NodeOperator, ProtocolConfig, RegistryState, Secp256k1KeyClaim},
    utils::registry::{
        derive_node_id, secp256k1_key_fingerprint, validate_secp256k1_pubkey, verify_node_pop,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterNodeArgs {
    pub new_index: u32,
    pub registration_salt: [u8; 32],
    pub ip: [u8; 4],
    pub port: u16,
    pub secp256k1_pubkey_compressed: [u8; 33],
    pub secp256k1_key_fingerprint: [u8; 32],
    pub pop_sig_r: [u8; 32],
    pub pop_sig_s: [u8; 32],
}

pub fn register_node(ctx: Context<RegisterNode>, args: RegisterNodeArgs) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let registry = &mut ctx.accounts.registry;
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
        args.registration_salt != [0u8; 32],
        MolphaError::InvalidRegistrationSalt
    );
    let (pub_key_x, pub_key_y) = validate_secp256k1_pubkey(&args.secp256k1_pubkey_compressed)?;
    let node_id = derive_node_id(
        ctx.accounts.node_owner.key(),
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

    let key_fingerprint = secp256k1_key_fingerprint(&args.secp256k1_pubkey_compressed);
    require!(
        args.secp256k1_key_fingerprint == key_fingerprint,
        MolphaError::InvalidSecp256k1KeyFingerprint
    );
    require!(
        !ctx.accounts.secp256k1_key_claim.claimed,
        MolphaError::NodeAlreadyRegistered
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
        config.minimum_node_deposit,
    )?;

    let owner_key = ctx.accounts.node_owner.key();
    ctx.accounts.node_operator.init(
        owner_key,
        args.new_index,
        args.ip,
        args.port,
        config.minimum_node_deposit,
        now,
        ctx.bumps.node_operator,
    );

    ctx.accounts.node.init(
        args.new_index,
        owner_key,
        pub_key_x,
        pub_key_y,
        ctx.bumps.node,
    );

    ctx.accounts.secp256k1_key_claim.init(
        key_fingerprint,
        args.secp256k1_pubkey_compressed,
        owner_key,
        node_id,
        ctx.bumps.secp256k1_key_claim,
    );

    registry.record_add_transition(now);

    emit!(NodeRegistered {
        node_id,
        new_index: args.new_index,
        owner: ctx.accounts.node_owner.key(),
        ip: args.ip,
        port: args.port,
        current_version: registry.current_version,
        security_deposit: config.minimum_node_deposit,
        registered_at: now,
    });

    Ok(())
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
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        has_one = authority @ MolphaError::UnauthorisedAuthority,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry.bump,
    )]
    pub registry: Box<Account<'info, RegistryState>>,

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
            registry.current_node_count.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub node: Box<Account<'info, Node>>,

    #[account(
        init_if_needed,
        payer = node_owner,
        space = Secp256k1KeyClaim::SPACE,
        seeds = [
            Secp256k1KeyClaim::SEED_PREFIX,
            args.secp256k1_key_fingerprint.as_ref(),
        ],
        bump
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
