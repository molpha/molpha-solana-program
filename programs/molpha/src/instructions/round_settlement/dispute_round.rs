use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};
use molpha_verifier::SchnorrSignature;

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    events::RoundDisputed,
    state::{
        Gateway, ProtocolConfig, Registry, RoundReceiptRecord, RoundRecordKind, RoundRecordStatus,
        Subscription,
    },
};

pub fn dispute_round<'info>(mut ctx: Context<DisputeRound<'info>>) -> Result<()> {
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let record = &ctx.accounts.round_record;
    require!(
        clock.slot.saturating_sub(record.recorded_slot)
            <= ctx.accounts.protocol_config.dispute_window_slots,
        MolphaError::DisputeWindowElapsed
    );

    let signature = SchnorrSignature {
        agg_sig_s: record.agg_sig_s,
        commitment: record.commitment,
        signers_bitmap: record.signers_bitmap,
    };
    let is_valid_sig = molpha_verifier::solana::verify_aggregate_over_hash_accounts(
        &ctx.accounts.registry.to_account_info(),
        signature,
        &record.attestation_message_hash,
        record.registry_version,
        ctx.remaining_accounts,
        ctx.program_id,
    )
    .map_err(map_attestation_accounts_err)?;

    require!(!is_valid_sig, MolphaError::DisputeSignatureValid);

    // x402 rounds are not refunded on dispute.
    let round_restored = match ctx.accounts.round_record.kind {
        RoundRecordKind::X402 => {
            require!(
                ctx.accounts.subscription.is_none(),
                MolphaError::WrongSubscriptionAccount
            );
            false
        }
        RoundRecordKind::Subscription => {
            restore_subscription_round(&mut ctx, now)?;
            true
        }
    };

    let config = &ctx.accounts.protocol_config;
    let slashed_bond = ctx.accounts.gateway_bond.amount;
    let bounty = (slashed_bond as u128)
        .saturating_mul(config.challenger_bounty_bps as u128)
        .checked_div(10_000)
        .unwrap_or(0) as u64;

    if bounty > 0 {
        transfer_from_gateway_bond(&ctx, ctx.accounts.challenger_usdc.to_account_info(), bounty)?;
    }
    let remainder = slashed_bond.saturating_sub(bounty);
    if remainder > 0 {
        transfer_from_gateway_bond(&ctx, ctx.accounts.treasury.to_account_info(), remainder)?;
    }

    let gateway = &mut ctx.accounts.gateway;
    gateway.slashed = true;

    let record = &mut ctx.accounts.round_record;
    record.status = RoundRecordStatus::Voided;

    emit!(RoundDisputed {
        round_record: record.key(),
        gateway: record.gateway,
        slashed_bond,
        challenger_bounty: bounty,
        round_restored,
    });

    Ok(())
}

fn transfer_from_gateway_bond<'info>(
    ctx: &Context<DisputeRound<'info>>,
    destination: AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    let authority = ctx.accounts.gateway.authority;
    let bump = [ctx.accounts.gateway.bump];
    let signer_seeds: &[&[u8]] = &[Gateway::SEED_PREFIX, authority.as_ref(), &bump];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.gateway_bond.to_account_info(),
                to: destination,
                authority: ctx.accounts.gateway.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
    )?;
    Ok(())
}

fn restore_subscription_round<'info>(
    ctx: &mut Context<DisputeRound<'info>>,
    now: i64,
) -> Result<()> {
    let subscription = ctx
        .accounts
        .subscription
        .as_mut()
        .ok_or_else(|| error!(MolphaError::WrongSubscriptionAccount))?;

    require!(
        subscription.owner.to_bytes() == ctx.accounts.round_record.consumer_id,
        MolphaError::WrongSubscriptionAccount
    );
    require!(
        subscription.is_active_at(now),
        MolphaError::WrongSubscriptionAccount
    );

    subscription.restore_round()
}

#[derive(Accounts)]
pub struct DisputeRound<'info> {
    #[account(mut)]
    pub challenger: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, round_record.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        constraint = round_record.status == RoundRecordStatus::Pending @ MolphaError::RoundNotPending,
    )]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidMint
    )]
    pub usdc_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = gateway,
        associated_token::token_program = token_program,
    )]
    pub gateway_bond: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = challenger,
        associated_token::mint = usdc_mint,
        associated_token::authority = challenger,
        associated_token::token_program = token_program,
    )]
    pub challenger_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,

    /// Subscription path (`record.kind == Subscription`). Must be `None` for x402 rounds.
    #[account(mut)]
    pub subscription: Option<Account<'info, Subscription>>,
}
