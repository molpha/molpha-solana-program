//! Gateway-funded pay-per-request settlement: debits gateway USDC float at the protocol price and
//! records the round like `settle_subscription_round`. Payment rail/ref metadata is attested only.

use crate::{
    error::MolphaError,
    events::X402RoundSettled,
    state::{
        Gateway, GatewayStatus, ProtocolConfig, Registry, RoundReceiptInitArgs, RoundReceiptRecord,
        RoundRecordKind,
    },
    utils::settlement::{validate_settlement_attestation, RoundRequest},
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

pub const MAX_PAYMENT_REF_LEN: usize = 64;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleX402RoundArgs {
    /// Rail-scoped payer identity for PDA seeds; not derivable on-chain.
    pub payer_id: [u8; 32],
    pub amount: u64,
    pub agg_sig: crate::state::AggregatedSig,
    pub value: [u8; 32],
    pub payment_rail: u16,
    /// Attested off-chain payment reference; not verified on-chain.
    pub payment_ref: Vec<u8>,
}

/// Price from request-time parameters only (not post-hoc signer count).
///
/// Charges for the whole selection, `signatures_required + redundancy_buffer`, because that is
/// the largest coalition finalization may credit. Every settled x402 round therefore funds its
/// own rewards, whatever subset of the selection ends up signing.
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
    ctx: Context<SettleX402Round<'info>>,
    round_request: RoundRequest,
    args: SettleX402RoundArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let registry = ctx.accounts.registry.load()?;
    let gateway = &ctx.accounts.gateway;
    let clock = Clock::get()?;
    let now_slot = clock.slot;

    let price = compute_x402_price(
        config,
        round_request.signatures_required,
        registry.redundancy_buffer,
    )?;
    require!(price == args.amount, MolphaError::AmountMismatch);

    require!(
        args.payment_ref.len() <= MAX_PAYMENT_REF_LEN,
        MolphaError::PaymentRefTooLong
    );

    let validated = validate_settlement_attestation(
        &registry,
        config,
        &round_request,
        &args.agg_sig,
        args.value,
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

    let reward_per_signature = config.reward_per_signature;

    ctx.accounts.round_record.init(RoundReceiptInitArgs {
        consumer_id: args.payer_id,
        selection_bitmap: validated.selection_bitmap,
        agg_sig: args.agg_sig,
        attestation_message_hash: validated.attestation_message_hash,
        gateway: gateway.key(),
        registry_version: round_request.registry_version,
        reward_per_signature,
        recorded_slot: now_slot,
        kind: RoundRecordKind::X402,
        bump: ctx.bumps.round_record,
    });

    emit!(X402RoundSettled {
        payer_id: args.payer_id,
        payment_rail: args.payment_rail,
        price,
        source_id: round_request.source_id,
        canonical_timestamp: round_request.canonical_timestamp,
        gateway: gateway.key(),
        signers_bitmap: ctx.accounts.round_record.signers_bitmap,
        credited_count: validated.credited_count,
        recorded_slot: now_slot,
        recorded_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(round_request: RoundRequest, args: SettleX402RoundArgs)]
pub struct SettleX402Round<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, round_request.registry_version.to_le_bytes().as_ref()],
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
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program,
    )]
    pub gateway_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,

    #[account(
        init,
        payer = authority,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::X402_SEED_PREFIX,
            args.payer_id.as_ref(),
            gateway.key().as_ref(),
            round_request.source_id.as_ref(),
            round_request.signatures_required.to_le_bytes().as_ref(),
            round_request.canonical_timestamp.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    pub system_program: Program<'info, System>,
}
