use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::MolphaError,
    events::GatewaySettlementDeposited,
    state::{Gateway, ProtocolConfig},
};

/// Pre-fund the gateway's external-rail settlement balance (agent-access spec v1 §5.2).
///
/// For x402 rounds the agent pays the gateway off-chain; `settle_agent_round` then debits
/// the round price from this balance. Kept separate from `bond` — the bond is slashable
/// dispute collateral, this is working capital. Withdrawal is Phase 1.5.
pub fn deposit_gateway_settlement(
    ctx: Context<DepositGatewaySettlement>,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, MolphaError::InvalidAmount);

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.authority_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.gateway_authority.to_account_info(),
            },
        ),
        amount,
    )?;

    let gateway = &mut ctx.accounts.gateway;
    gateway.settlement_balance = gateway
        .settlement_balance
        .checked_add(amount)
        .ok_or(MolphaError::InvalidAmount)?;

    emit!(GatewaySettlementDeposited {
        gateway: gateway.key(),
        amount,
        new_balance: gateway.settlement_balance,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct DepositGatewaySettlement<'info> {
    pub gateway_authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump = gateway.bump,
        constraint = gateway.authority == gateway_authority.key() @ MolphaError::UnauthorisedGateway,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(
        mut,
        constraint = authority_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = authority_usdc.owner == gateway_authority.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub authority_usdc: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
