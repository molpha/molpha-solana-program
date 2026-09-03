use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::{validate_key_and_verify_pop, NodePopError};
use solana_keccak_hasher::hashv;

use crate::{
    error::MolphaError,
    events::NodeRegistered,
    state::{
        Node, ProtocolConfig, Registry, RegistryState, Secp256k1KeyClaim, MAX_REGISTRY_NODES,
        NODE_DEPOSITS_SEED,
    },
    utils::registry::derive_node_id,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterNodeArgs {
    pub registration_salt: [u8; 32],
    pub ip: [u8; 4],
    pub port: u16,
    pub secp256k1_pubkey_compressed: [u8; 33],
    pub pop_sig_r: [u8; 32],
    pub pop_sig_s: [u8; 32],
}

pub fn register_node(ctx: Context<RegisterNode>, args: RegisterNodeArgs) -> Result<()> {
    require!(
        args.ip != [0, 0, 0, 0] && args.port != 0,
        MolphaError::InvalidNodeEndpoint
    );

    let node_index = {
        let registry = ctx.accounts.current_registry.load()?;
        require!(
            u32::from(registry.node_count) < MAX_REGISTRY_NODES as u32,
            MolphaError::NodeCapExceeded
        );
        registry.node_count
    };

    require!(
        args.registration_salt != [0u8; 32],
        MolphaError::InvalidRegistrationSalt
    );
    let owner = ctx.accounts.node_owner.key();
    let node_id = derive_node_id(
        owner,
        &args.secp256k1_pubkey_compressed,
        &args.registration_salt,
    );
    let (pub_key_x, pub_key_y) = validate_key_and_verify_pop(
        &ctx.program_id.to_bytes(),
        &node_id,
        &args.secp256k1_pubkey_compressed,
        &args.pop_sig_r,
        &args.pop_sig_s,
    )
    .map_err(|error| match error {
        NodePopError::InvalidPublicKey => error!(MolphaError::PubkeyNotOnCurve),
        NodePopError::InvalidProof => error!(MolphaError::InvalidPoP),
    })?;

    // Principal goes into the node-deposit vault, never the treasury, so no reward or bounty
    // payout can reach it.
    let deposit = ctx.accounts.protocol_config.minimum_node_deposit;
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.owner_usdc.to_account_info(),
                to: ctx.accounts.node_deposit_vault.to_account_info(),
                authority: ctx.accounts.node_owner.to_account_info(),
            },
        ),
        deposit,
    )?;
    let config = &mut ctx.accounts.protocol_config;
    config.total_node_deposits = config
        .total_node_deposits
        .checked_add(deposit)
        .ok_or(MolphaError::AmountMismatch)?;

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
    ctx.accounts
        .secp256k1_key_claim
        .init(ctx.accounts.node.key(), ctx.bumps.secp256k1_key_claim);

    let current = ctx.accounts.current_registry.load()?;
    let mut next = ctx.accounts.new_registry.load_init()?;
    next.copy_from_previous(&current, ctx.bumps.new_registry);
    let appended_index = next.append(ctx.accounts.node.key())?;
    require!(appended_index == node_index, MolphaError::InvalidNodeIndex);
    ctx.accounts.registry_state.advance_version();

    emit!(NodeRegistered {
        node: ctx.accounts.node.key(),
        index: u32::from(node_index),
        registry_version: ctx.accounts.registry_state.next_version,
        owner,
        ip: args.ip,
        port: args.port,
        security_deposit: ctx.accounts.protocol_config.minimum_node_deposit,
        registered_at: now,
    });

    Ok(())
}

pub fn hash_seed(input: &[u8]) -> [u8; 32] {
    let hash_result = hashv(&[input]);
    hash_result.to_bytes()
}

#[derive(Accounts)]
#[instruction(args: RegisterNodeArgs)]
pub struct RegisterNode<'info> {
    pub authority: Signer<'info>,

    #[account(mut)]
    pub node_owner: Signer<'info>,

    #[account(
        mut,
        has_one = authority,
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
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
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
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = node_owner,
        associated_token::token_program = token_program,
    )]
    pub owner_usdc: Box<Account<'info, TokenAccount>>,

    /// CHECK: seeds-validated deposit-vault authority; holds no data.
    #[account(
        seeds = [NODE_DEPOSITS_SEED],
        bump = protocol_config.node_deposits_bump,
    )]
    pub node_deposits: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = node_deposits,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Box<Account<'info, TokenAccount>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
