use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::{error::MolphaError, events::SubscriptionVaultWithdrawn, state::ProtocolConfig};

pub fn admin_withdraw_subscription_vault(
    ctx: Context<AdminWithdrawSubscriptionVault>,
) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.authority.key(),
        ctx.accounts.protocol_config.authority,
        MolphaError::UnauthorisedAuthority
    );

    let amount = ctx.accounts.treasury.amount;
    if amount == 0 {
        return Ok(());
    }

    let decimals = ctx.accounts.usdc_mint.decimals;
    let bump = [ctx.accounts.protocol_config.bump];
    let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.treasury.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.protocol_config.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
        decimals,
    )?;

    emit!(SubscriptionVaultWithdrawn {
        authority: ctx.accounts.authority.key(),
        destination: ctx.accounts.destination.key(),
        amount,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct AdminWithdrawSubscriptionVault<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc
    )]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == protocol_config.usdc_mint @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        constraint = destination.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = destination.owner == authority.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    pub authority: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}
