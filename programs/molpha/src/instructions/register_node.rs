use crate::error::NodeRegistryError;
use crate::events::NodeAdded;
use crate::state::{Node, ProtocolConfig};
use anchor_lang::prelude::*;
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
    node.init(
        ctx.accounts.authority.key(),
        node_pubkey,
        evm_pubkey.clone(),
        url.clone(),
    )?;
    node.security_deposit = security_deposit_amount;

    // Emit event
    emit!(NodeAdded {
        node: node_pubkey,
        evm_pubkey: evm_pubkey,
        authority: ctx.accounts.authority.key(),
        security_deposit: security_deposit_amount,
        url: url.clone(),
        added_at: Clock::get()?.unix_timestamp,
    });

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
}
