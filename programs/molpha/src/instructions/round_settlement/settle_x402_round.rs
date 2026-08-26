//! `settle_x402_round` — unified gateway-funded settlement for every pay-per-request rail.
//!
//! A bonded gateway debits its own USDC ("float") into the protocol treasury at the
//! protocol-quoted price and records how it was paid as attested metadata (`payment_rail`,
//! `payment_ref`). The program enforces exactly what it can enforce: the signer is the bonded
//! gateway authority, the treasury receives the exact protocol price, and the round identity is
//! unique (PDA init). Which rail was used, the payment reference, and whether the payer actually
//! paid are gateway-claimed and never verified on-chain — Solana cannot verify a Base
//! transaction, and pretending otherwise would be theater. Node revenue is the invariant this
//! instruction protects, and it is fully preserved: it is enforced identically to
//! `settle_subscription_round`.

use crate::{
    error::{map_attestation_err, MolphaError},
    events::X402RoundSettled,
    state::{
        AggregatedSig, Gateway, ProtocolConfig, Registry, RegistryState, RoundReceiptRecord,
        RoundRecordKind,
    },
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::{
    bitmap::{bitmap_is_subset, bitmap_popcount},
    compute_message_hash, derive_selection_bitmap, AttestationPayload,
};

/// Cap on `payment_ref`: a 64-byte Solana signature or any 32-byte EVM tx hash fits comfortably.
pub const MAX_PAYMENT_REF_LEN: usize = 64;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SettleX402RoundArgs {
    /// Rail-scoped payer identity: `keccak256(rail_id_be ‖ payer_address_bytes)`. Needed in the
    /// receipt and PDA seeds (replay scoping, charge accounting) — not derivable on-chain.
    pub payer_id: [u8; 32],
    /// Expected price; must equal the derived price exactly (§ below).
    pub amount: u64,
    pub source_id: [u8; 32],
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub registry_version: u32,
    pub agg_sig: AggregatedSig,
    pub value: [u8; 32],
    /// Payment rail registry id (gateway-level, not on-chain governed). 0 = none/subscription,
    /// 1 = solana, 2 = base.
    pub payment_rail: u16,
    /// Opaque payment reference (tx hash, facilitator receipt, ...); attested, never verified.
    pub payment_ref: Vec<u8>,
}

/// Price from request-time parameters only (never the post-hoc bitmap popcount), so external
/// rails requiring a locked price (x402 `exact`) stay applicable.
pub fn compute_x402_price(config: &ProtocolConfig, signatures_required: u8) -> Result<u64> {
    config
        .x402_round_base
        .checked_add(
            (signatures_required as u64)
                .checked_mul(config.x402_round_per_sig)
                .ok_or(MolphaError::AmountMismatch)?,
        )
        .ok_or_else(|| error!(MolphaError::AmountMismatch))
}

pub fn settle_x402_round<'info>(
    ctx: Context<SettleX402Round<'info>>,
    args: SettleX402RoundArgs,
) -> Result<()> {
    let config = &ctx.accounts.protocol_config;
    let registry = ctx.accounts.registry.load()?;
    let gateway = &ctx.accounts.gateway;
    let clock = Clock::get()?;
    let now_slot = clock.slot;

    require!(
        args.registry_version == ctx.accounts.registry_state.current_version
            && registry.version == args.registry_version,
        MolphaError::InvalidRegistryVersion
    );

    let price = compute_x402_price(config, args.signatures_required)?;
    require!(price == args.amount, MolphaError::AmountMismatch);

    require!(
        args.payment_ref.len() <= MAX_PAYMENT_REF_LEN,
        MolphaError::PaymentRefTooLong
    );

    // Selection subset + quorum bounds on the embedded bitmap (Schnorr verify stays on dispute).
    let expected_selection = derive_selection_bitmap(
        &args.source_id,
        args.registry_version,
        args.canonical_timestamp as u64,
        u32::from(registry.node_count),
        args.signatures_required,
        registry.redundancy_buffer,
    )
    .map_err(map_attestation_err)?;

    let selection_mismatch = !bitmap_is_subset(&args.agg_sig.signers_bitmap, &expected_selection);
    require!(
        !selection_mismatch,
        MolphaError::SignersNotSubsetOfSelection
    );

    let count = bitmap_popcount(&args.agg_sig.signers_bitmap);
    require!(
        count >= u32::from(args.signatures_required),
        MolphaError::QuorumBelowThreshold
    );
    require!(
        count <= u32::from(args.signatures_required) + u32::from(registry.redundancy_buffer),
        MolphaError::CreditedExceedsSelection
    );

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
                authority: ctx.accounts.gateway_authority.to_account_info(),
            },
        ),
        price,
    )?;

    let reward_per_signature = config.reward_per_signature;
    let gateway_fee = config.gateway_fee;

    let attestation_message_hash = compute_message_hash(
        &AttestationPayload {
            value: args.value,
            source_id: args.source_id,
            signatures_required: args.signatures_required,
            registry_version: args.registry_version,
            canonical_timestamp: args.canonical_timestamp,
        },
        args.agg_sig.signers_bitmap,
    );

    // Record. The price surplus over node + gateway credits is protocol revenue, implicit in the
    // treasury (identical to subscription revenue). Payment rail / ref / payer_id remain instruction
    // args and event fields only — they are not stored on the slim round receipt.
    let record = &mut ctx.accounts.round_record;
    record.signers_bitmap = args.agg_sig.signers_bitmap;
    record.agg_sig_s = args.agg_sig.agg_sig_s;
    record.commitment = args.agg_sig.commitment;
    record.attestation_message_hash = attestation_message_hash;
    record.gateway = gateway.key();
    record.registry_version = args.registry_version;
    record.reward_per_signature = reward_per_signature;
    record.gateway_fee = gateway_fee;
    record.recorded_slot = now_slot;
    record.kind = RoundRecordKind::X402;
    record.bump = ctx.bumps.round_record;

    emit!(X402RoundSettled {
        payer_id: args.payer_id,
        payment_rail: args.payment_rail,
        price,
        source_id: args.source_id,
        timestamp: args.canonical_timestamp,
        gateway: gateway.key(),
        signers_bitmap: record.signers_bitmap,
        credited_count: count,
        recorded_slot: now_slot,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: SettleX402RoundArgs)]
pub struct SettleX402Round<'info> {
    #[account(mut)]
    pub gateway_authority: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Gateway::SEED_PREFIX, gateway_authority.key().as_ref()],
        bump = gateway.bump,
        constraint = !gateway.slashed && gateway.bond >= protocol_config.gateway_bond_min @ MolphaError::GatewayNotBonded,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        constraint = usdc_mint.key() == protocol_config.usdc_mint @ MolphaError::InvalidTreasury
    )]
    pub usdc_mint: Box<Account<'info, token::Mint>>,

    #[account(
        mut,
        constraint = gateway_usdc.mint == usdc_mint.key() @ MolphaError::InvalidOwnerUsdc,
        constraint = gateway_usdc.owner == gateway_authority.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub gateway_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.mint == usdc_mint.key() @ MolphaError::InvalidTreasury,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,

    #[account(
        init,
        payer = gateway_authority,
        space = RoundReceiptRecord::SPACE,
        seeds = [
            RoundReceiptRecord::X402_SEED_PREFIX,
            args.payer_id.as_ref(),
            gateway.key().as_ref(),
            args.source_id.as_ref(),
            args.signatures_required.to_le_bytes().as_ref(),
            args.canonical_timestamp.to_le_bytes().as_ref(),
        ],
        bump
    )]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    pub system_program: Program<'info, System>,
}
