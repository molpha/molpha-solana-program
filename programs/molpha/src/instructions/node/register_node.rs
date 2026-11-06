//! Admit a node or a provider signer to the registry. Both are co-signed by the protocol
//! authority, prove possession of the signing key the same way, reserve it with a `KeyXClaim`, and
//! enter the registry in the version this publishes.
//!
//! - A node (the `node` account, `MemberEndpoint::Node`) locks the minimum deposit and takes the
//!   lowest free node slot.
//! - A provider (the `provider` account, `MemberEndpoint::Provider`) locks nothing, since nothing
//!   slashes a provider, takes the lowest free provider slot from 1 and gets the next provider id.
//!   Its record joins the provider table; it signs at bit `64 + slot`.

use crate::{
    error::MolphaError,
    events::{NodeRegistered, ProviderRegistered},
    state::{
        compressed_key_x, derive_node_id, KeyXClaim, Node, ProtocolConfig, Provider, Registry,
        RegistryState, RegistryTransition,
    },
    utils::{pop_error, require_valid_p2p_key},
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::validate_key_and_verify_pop;

/// Where the gateway reaches the member. Its variant names the kind being registered and must
/// match the account passed.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum MemberEndpoint {
    /// A node's public address.
    Node { ip: [u8; 4], port: u16 },
    /// A provider's ed25519 libp2p identity: its signer dials the gateway, nothing dials it.
    Provider { p2p_key: [u8; 32] },
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterNodeArgs {
    pub registration_salt: [u8; 32],
    pub endpoint: MemberEndpoint,
    pub secp256k1_pubkey_compressed: [u8; 33],
    /// Proof of possession, `MOLPHA_NODE_POP_V1` over the node id
    /// `keccak256("MOLPHA_NODE_ID_V1" ‖ node_owner ‖ key ‖ salt)`, for a provider too.
    pub pop_sig_r: [u8; 32],
    pub pop_sig_s: [u8; 32],
}

pub fn register_node(ctx: Context<RegisterNode>, args: RegisterNodeArgs) -> Result<()> {
    match (&args.endpoint, &ctx.accounts.node, &ctx.accounts.provider) {
        (MemberEndpoint::Node { ip, port }, Some(_), None) => {
            require!(
                *ip != [0, 0, 0, 0] && *port != 0,
                MolphaError::InvalidNodeEndpoint
            );
        }
        (MemberEndpoint::Provider { p2p_key }, None, Some(_)) => require_valid_p2p_key(p2p_key)?,
        (_, Some(_), None) | (_, None, Some(_)) => return err!(MolphaError::MemberKindMismatch),
        _ => return err!(MolphaError::ExactlyOneMember),
    }

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
    let key = validate_key_and_verify_pop(
        &ctx.program_id.to_bytes(),
        &node_id,
        &args.secp256k1_pubkey_compressed,
        &args.pop_sig_r,
        &args.pop_sig_s,
    )
    .map_err(pop_error)?;

    let now = Clock::get()?.unix_timestamp;
    match args.endpoint {
        MemberEndpoint::Node { ip, port } => register_node_member(ctx, key, ip, port, now),
        MemberEndpoint::Provider { p2p_key } => register_provider_member(ctx, key, p2p_key, now),
    }
}

fn register_node_member(
    ctx: Context<RegisterNode>,
    key: ([u8; 32], [u8; 32]),
    ip: [u8; 4],
    port: u16,
    now: i64,
) -> Result<()> {
    let accounts = ctx.accounts;
    let owner = accounts.node_owner.key();
    let (Some(owner_usdc), Some(node_deposit_vault)) =
        (&accounts.owner_usdc, &accounts.node_deposit_vault)
    else {
        return err!(MolphaError::NodeDepositAccountsMissing);
    };

    // Principal goes into the node-deposit vault, never the treasury, so no reward or bounty
    // payout can reach it.
    let deposit = accounts.protocol_config.minimum_node_deposit;
    token::transfer(
        CpiContext::new(
            accounts.token_program.key(),
            Transfer {
                from: owner_usdc.to_account_info(),
                to: node_deposit_vault.to_account_info(),
                authority: accounts.node_owner.to_account_info(),
            },
        ),
        deposit,
    )?;

    let node = accounts.node.as_mut().unwrap();
    node.init(
        owner,
        key.0,
        key.1,
        ip,
        port,
        deposit,
        now,
        ctx.bumps.node.unwrap(),
    );
    let node_key = node.key();
    accounts.key_x_claim.init(node_key, ctx.bumps.key_x_claim);

    let grace_active_until = accounts.protocol_config.get_grace_active_until(now)?;
    let index = accounts.registry_state.insert_node(
        &RegistryTransition {
            current: &accounts.current_registry,
            next: &accounts.new_registry,
            payer: &accounts.node_owner,
            system_program: &accounts.system_program,
            grace_active_until,
            now,
            bump: ctx.bumps.new_registry,
        },
        node_key,
        key,
    )?;

    emit!(NodeRegistered {
        node: node_key,
        index,
        registry_version: accounts.registry_state.current_version,
        owner,
        ip,
        port,
        security_deposit: deposit,
        registered_at: now,
    });
    Ok(())
}

fn register_provider_member(
    ctx: Context<RegisterNode>,
    key: ([u8; 32], [u8; 32]),
    p2p_key: [u8; 32],
    now: i64,
) -> Result<()> {
    let accounts = ctx.accounts;
    let authority = accounts.node_owner.key();
    let provider_key = accounts.provider.as_ref().unwrap().key();

    // The `provider` PDA was derived from `next_provider_id` before this runs; take it now.
    let id = accounts.registry_state.take_provider_id()?;
    accounts
        .key_x_claim
        .init(provider_key, ctx.bumps.key_x_claim);

    let grace_active_until = accounts.protocol_config.get_grace_active_until(now)?;
    let slot = accounts.registry_state.insert_provider(
        &RegistryTransition {
            current: &accounts.current_registry,
            next: &accounts.new_registry,
            payer: &accounts.node_owner,
            system_program: &accounts.system_program,
            grace_active_until,
            now,
            bump: ctx.bumps.new_registry,
        },
        provider_key,
        key,
    )?;
    let slot = u8::try_from(slot).map_err(|_| error!(MolphaError::ProviderSlotsFull))?;

    accounts.provider.as_mut().unwrap().init(
        id,
        authority,
        key,
        p2p_key,
        slot,
        ctx.bumps.provider.unwrap(),
    );

    emit!(ProviderRegistered {
        provider: provider_key,
        id,
        authority,
        slot,
        registry_version: accounts.registry_state.current_version,
        pubkey_x: key.0,
        pubkey_y: key.1,
        p2p_key,
        registered_at: now,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(args: RegisterNodeArgs)]
pub struct RegisterNode<'info> {
    /// The protocol authority: every member, node or provider, is admitted with its signature.
    pub authority: Signer<'info>,

    /// The node owner, or the provider authority. Pays the rent of the member, its key claim and
    /// the registry version, and (a node) the deposit.
    #[account(mut)]
    pub node_owner: Signer<'info>,

    #[account(
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
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump = current_registry.load()?.bump,
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

    /// The node being registered; omit it for a provider.
    #[account(
        init,
        payer = node_owner,
        space = Node::SPACE,
        seeds = [Node::SEED_PREFIX, node_owner.key().as_ref()],
        bump,
    )]
    pub node: Option<Box<Account<'info, Node>>>,

    /// The provider being registered, at the next provider id; omit it for a node.
    #[account(
        init,
        payer = node_owner,
        space = Provider::SPACE,
        seeds = [Provider::SEED_PREFIX, registry_state.next_provider_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub provider: Option<Box<Account<'info, Provider>>>,

    /// Reserves the key's x coordinate, so a duplicate or negated key (a node's or a provider's)
    /// cannot be registered.
    #[account(
        init,
        payer = node_owner,
        space = KeyXClaim::SPACE,
        seeds = [KeyXClaim::SEED_PREFIX, compressed_key_x(&args.secp256k1_pubkey_compressed)],
        bump,
    )]
    pub key_x_claim: Box<Account<'info, KeyXClaim>>,

    /// The node owner's USDC, which pays the deposit; omit it for a provider.
    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = node_owner,
        associated_token::token_program = token_program,
    )]
    pub owner_usdc: Option<Box<Account<'info, TokenAccount>>>,

    /// The node-deposit vault; omit it for a provider.
    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Option<Box<Account<'info, TokenAccount>>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
