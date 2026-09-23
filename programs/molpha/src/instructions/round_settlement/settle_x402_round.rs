//! Gateway-funded pay-per-request settlement: debits gateway USDC float at the protocol price and
//! settles the round like `settle_subscription_round`. Payment rail/ref metadata is attested only.

use crate::{
    error::MolphaError,
    events::X402RoundSettled,
    state::{Gateway, GatewayStatus, ProtocolConfig, Registry, RoundReplayKind, RoundReplayMarker},
    utils::{
        attestation::{validate_attestation, NodeAccountLayout},
        settlement::credit_round_rewards,
    },
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::Attestation;

pub const MAX_PAYMENT_REF_LEN: usize = 64;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleX402RoundArgs {
    /// Rail-scoped payer identity for PDA seeds; not derivable on-chain.
    pub payer_id: [u8; 32],
    pub amount: u64,
    pub payment_rail: u16,
    /// Attested off-chain payment reference; not verified on-chain.
    pub payment_ref: Vec<u8>,
}

/// Price from request-time parameters only (not post-hoc signer count).
///
/// Charges for the whole selection, `signatures_required + redundancy_buffer`, because that is
/// the largest coalition settlement may credit. Every settled x402 round therefore funds its own
/// rewards, whatever subset of the selection ends up signing.
pub fn compute_x402_price(
    config: &ProtocolConfig,
    signatures_required: u8,
    redundancy_buffer: u8,
) -> Result<u64> {
    let credited_ceiling = u64::from(signatures_required)
        .checked_add(u64::from(redundancy_buffer))
        .ok_or(MolphaError::AmountMismatch)?;
    config
        .x402_round_base
        .checked_add(
            credited_ceiling
                .checked_mul(config.reward_per_signature)
                .ok_or(MolphaError::AmountMismatch)?,
        )
        .ok_or_else(|| error!(MolphaError::AmountMismatch))
}

pub fn settle_x402_round<'info>(
    ctx: Context<'info, SettleX402Round<'info>>,
    attestation: Attestation,
    args: SettleX402RoundArgs,
) -> Result<()> {
    let gateway = &ctx.accounts.gateway;
    let clock = Clock::get()?;
    let now_slot = clock.slot;

    let redundancy_buffer = ctx.accounts.registry.load()?.redundancy_buffer;
    let price = compute_x402_price(
        &ctx.accounts.config,
        attestation.payload.signatures_required,
        redundancy_buffer,
    )?;
    require!(price == args.amount, MolphaError::AmountMismatch);

    require!(
        args.payment_ref.len() <= MAX_PAYMENT_REF_LEN,
        MolphaError::PaymentRefTooLong
    );

    validate_attestation(
        ctx.remaining_accounts,
        NodeAccountLayout::SignersThenOthers,
        &ctx.accounts.registry,
        &ctx.accounts.config,
        &attestation,
        clock.unix_timestamp,
    )?;

    require!(
        ctx.accounts.gateway_usdc.amount >= price,
        MolphaError::InsufficientGatewayFloat
    );

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.gateway_usdc.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
        ),
        price,
    )?;

    // Anchor does not refresh deserialized token accounts after a CPI. Include the payment that
    // just landed before checking whether the treasury backs the new reward liability.
    ctx.accounts.treasury.reload()?;

    credit_round_rewards(
        &ctx.accounts.registry,
        &mut ctx.accounts.config,
        &ctx.accounts.treasury,
        ctx.remaining_accounts,
        &attestation,
        ctx.accounts.round_replay.key(),
    )?;

    ctx.accounts.round_replay.init(
        RoundReplayMarker::round_id(RoundReplayKind::X402, &attestation, &args.payer_id),
        attestation.payload.canonical_timestamp,
        ctx.bumps.round_replay,
    );

    emit!(X402RoundSettled {
        payer_id: args.payer_id,
        payment_rail: args.payment_rail,
        price,
        source_id: attestation.payload.source_id,
        canonical_timestamp: attestation.payload.canonical_timestamp,
        gateway: gateway.key(),
        signers_bitmap: attestation.signature.signers_bitmap,
        recorded_slot: now_slot,
        recorded_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(attestation: Attestation, args: SettleX402RoundArgs)]
pub struct SettleX402Round<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = config.bump,
    )]
    pub config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, attestation.payload.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        has_one = authority,
        constraint = gateway.status == GatewayStatus::Active @ MolphaError::GatewayNotActive,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program,
    )]
    pub gateway_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        init,
        payer = authority,
        space = RoundReplayMarker::SPACE,
        seeds = [
            RoundReplayMarker::SEED_PREFIX,
            gateway.key().as_ref(),
            &RoundReplayMarker::round_id(RoundReplayKind::X402, &attestation, &args.payer_id),
        ],
        bump,
    )]
    pub round_replay: Account<'info, RoundReplayMarker>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
