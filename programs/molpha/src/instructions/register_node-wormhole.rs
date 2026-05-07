use crate::error::NodeRegistryError;
use crate::events::NodeAdded;
use crate::state::{Node, NodeRegistrationMessage, ProtocolConfig};
use anchor_lang::prelude::*;
use libsecp256k1::{PublicKey, PublicKeyFormat};
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;
use anchor_spl::{
    associated_token::AssociatedToken,
    token,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

pub fn register_node(
    ctx: Context<RegisterNode>,
    node_pubkey: Pubkey,
    evm_pubkey: Vec<u8>,
    url: String,
) -> Result<()> {
    require!(
        node_pubkey != Pubkey::default(),
        NodeRegistryError::ZeroPubkey
    );
    require!(!url.is_empty(), NodeRegistryError::InvalidURL);
    require!(evm_pubkey.len() == 33, NodeRegistryError::InvalidEvmPubkey);

    let protocol_config = &ctx.accounts.protocol_config;
    let security_deposit_amount = protocol_config.node_security_deposit;
    let user_token_account = &ctx.accounts.node_operator_token_account;
    let node_security_deposit_account = &ctx.accounts.node_security_deposit_account;
    let token_program = &ctx.accounts.token_program;
    let authority = &ctx.accounts.authority;

    // Check if deposit is required and sufficient
    // if security_deposit_amount > 0 {
    require!(
        ctx.accounts.node_operator_token_account.amount >= security_deposit_amount,
        NodeRegistryError::InsufficientSecurityDeposit
    );

    let deposit_cpi_ctx = CpiContext::new(
        token_program.to_account_info(),
        token::Transfer {
            from: user_token_account.to_account_info(),
            to: node_security_deposit_account.to_account_info(),
            authority: authority.to_account_info(),
        },
    );
    token::transfer(deposit_cpi_ctx, security_deposit_amount)?;

    // Create the node PDA account
    let node = &mut ctx.accounts.node;

    let pk = PublicKey::parse_slice(&evm_pubkey, Some(PublicKeyFormat::Compressed))
        .map_err(|_| error!(NodeRegistryError::InvalidEvmPubkey))?;
    let uncompressed = pk.serialize(); // 65 bytes: 0x04 || x || y
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(&uncompressed[1..33]);
    y.copy_from_slice(&uncompressed[33..65]);

    node.init(
        ctx.accounts.authority.key(),
        node_pubkey,
        (x, y),
        url.clone(),
    )?;
    node.security_deposit = security_deposit_amount;

    let timestamp = Clock::get()?.unix_timestamp;

    // Emit event
    emit!(NodeAdded {
        node: node_pubkey,
        evm_pubkey: evm_pubkey.clone(),
        authority: ctx.accounts.authority.key(),
        security_deposit: security_deposit_amount,
        url: url.clone(),
        added_at: timestamp,
    });

    // Send cross-chain message via Wormhole if bridge program is configured and chains are configured
    if protocol_config.wormhole_bridge_program != Pubkey::default()
        && !protocol_config.target_chain_ids.is_empty()
    {
        send_wormhole_message(&ctx, evm_pubkey, security_deposit_amount, timestamp)?;
    }

    Ok(())
}

/// Send a Wormhole message to notify EVM chains about the new node registration
fn send_wormhole_message(
    ctx: &Context<RegisterNode>,
    evm_pubkey: Vec<u8>,
    security_deposit: u64,
    registered_at: i64,
) -> Result<()> {
    // Create the message payload
    let message = NodeRegistrationMessage::new(evm_pubkey, security_deposit, registered_at)?;
    let payload = message.to_bytes()?;

    // Use timestamp as nonce for uniqueness
    let nonce =
        (registered_at as u32).wrapping_add(ctx.accounts.authority.key().to_bytes()[0] as u32);

    // Wormhole instruction data: post_message(nonce, payload, consistency_level)
    // Instruction discriminator for post_message is [0x01]
    // Consistency level: Finalized = 1
    let mut instruction_data = vec![0x01]; // post_message discriminator
    instruction_data.extend_from_slice(&nonce.to_le_bytes());
    instruction_data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    instruction_data.extend_from_slice(&payload);
    instruction_data.push(1); // Finalized consistency level

    // Build Wormhole post_message instruction
    let wormhole_ix = Instruction {
        program_id: ctx.accounts.wormhole_bridge_program.key(),
        accounts: vec![
            AccountMeta::new(ctx.accounts.wormhole_bridge.key(), false),
            AccountMeta::new(ctx.accounts.wormhole_message.key(), true),
            AccountMeta::new_readonly(ctx.accounts.wormhole_emitter.key(), true),
            AccountMeta::new(ctx.accounts.wormhole_sequence.key(), false),
            AccountMeta::new(ctx.accounts.authority.key(), true),
            AccountMeta::new(ctx.accounts.wormhole_fee_collector.key(), false),
            AccountMeta::new_readonly(ctx.accounts.clock.key(), false),
            AccountMeta::new_readonly(ctx.accounts.rent.key(), false),
            AccountMeta::new_readonly(ctx.accounts.system_program.key(), false),
        ],
        data: instruction_data,
    };

    // Derive emitter PDA seeds
    let protocol_seeds = &[
        ProtocolConfig::SEED_PREFIX,
        &[ctx.accounts.protocol_config.bump],
    ];

    // Invoke Wormhole Core Bridge to post message
    invoke_signed(
        &wormhole_ix,
        &[
            ctx.accounts.wormhole_bridge_program.to_account_info(),
            ctx.accounts.wormhole_bridge.to_account_info(),
            ctx.accounts.wormhole_message.to_account_info(),
            ctx.accounts.wormhole_emitter.to_account_info(),
            ctx.accounts.wormhole_sequence.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.wormhole_fee_collector.to_account_info(),
            ctx.accounts.clock.to_account_info(),
            ctx.accounts.rent.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
        &[protocol_seeds],
    )?;

    Ok(())
}

#[derive(Accounts)]
#[instruction(node_pubkey: Pubkey)]
pub struct RegisterNode<'info> {
    #[account(
        init,
        payer = authority,
        space = Node::SPACE,
        seeds = [Node::SEED_PREFIX, node_pubkey.as_ref()],
        bump
    )]
    pub node: Account<'info, Node>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// The underlying token mint (from protocol config)
    #[account(
        mut,
        constraint = underlying_token.key() == protocol_config.underlying_token
    )]
    pub underlying_token: InterfaceAccount<'info, Mint>,

    /// Node operator's token account (source of security deposit)
    #[account(
        mut,
        associated_token::mint = underlying_token,
        associated_token::authority = authority,
    )]
    pub node_operator_token_account: InterfaceAccount<'info, TokenAccount>,

    /// Node security deposit token account (destination, PDA owned by node)
    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = underlying_token,
        associated_token::authority = node,
    )]
    pub node_security_deposit_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,

    // ========== Wormhole Cross-Chain Messaging Accounts ==========
    // These accounts are required when Wormhole bridge is configured in protocol_config
    /// CHECK: Wormhole Core Bridge program
    #[account(
        constraint = wormhole_bridge_program.key() == protocol_config.wormhole_bridge_program
            || protocol_config.wormhole_bridge_program == Pubkey::default()
    )]
    pub wormhole_bridge_program: UncheckedAccount<'info>,

    /// CHECK: Wormhole bridge config account
    #[account(mut)]
    pub wormhole_bridge: UncheckedAccount<'info>,

    /// CHECK: Wormhole message account (will be created by Wormhole)
    #[account(mut)]
    pub wormhole_message: Signer<'info>,

    /// CHECK: Wormhole emitter account - protocol_config PDA serves as emitter
    #[account(
        constraint = wormhole_emitter.key() == protocol_config.key()
    )]
    pub wormhole_emitter: UncheckedAccount<'info>,

    /// CHECK: Wormhole sequence account
    #[account(mut)]
    pub wormhole_sequence: UncheckedAccount<'info>,

    /// CHECK: Wormhole fee collector
    #[account(mut)]
    pub wormhole_fee_collector: UncheckedAccount<'info>,

    /// Clock sysvar for timestamp
    pub clock: Sysvar<'info, Clock>,

    /// Rent sysvar
    pub rent: Sysvar<'info, Rent>,
}
