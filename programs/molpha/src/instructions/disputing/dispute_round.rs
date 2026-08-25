//! `dispute_round` — the only place Schnorr verification runs.
//!
//! A challenger supplies the committed `AggregatedSig` (preimage of `record.agg_sig_hash`) and
//! forces verification against the registry keys selected by the signature's own
//! `signers_bitmap`. Because the credited set *is* that bitmap and the scheme is a plain-sum
//! coalition key, an over-credited (inflated) set cannot produce a valid aggregate (§1.1).
//!
//! - Invalid selection bitmap, or invalid signature over the credited set
//!   (fabricated round / committed garbage) -> void the round and slash `gateway.bond`
//!   from the gateway bond ATA, paying the challenger `challenger_bounty_bps` and sending
//!   the remainder to treasury. Subscription rounds restore `used_rounds`. x402 rounds are
//!   **not refunded** — the settled `price` came from the gateway's own float, and the gateway
//!   is the party being slashed; refunding it would return the fraud proceeds to the fraudster.
//!   The payer's recourse for a voided x402 round is bilateral (off-chain, against the gateway),
//!   using the round's `payer_id` / `payment_rail` / `payment_ref` plus this event as evidence.
//! - Valid selection bitmap and valid signature -> no fraud; dispute fails
//!   (`DisputeSignatureValid`).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::{bitmap::bitmap_is_subset, SchnorrSignature};

use crate::{
    error::{map_attestation_accounts_err, map_attestation_err, MolphaError},
    events::RoundDisputed,
    state::{
        hash_agg_sig, AggregatedSig, Gateway, ProtocolConfig, Registry, RoundReceiptRecord,
        RoundRecordKind, RoundRecordStatus, Subscription,
    },
};

pub fn dispute_round<'info>(
    mut ctx: Context<DisputeRound<'info>>,
    agg_sig: AggregatedSig,
) -> Result<()> {
    // let (selection_mismatch, is_valid_sig) = {
    let record = &ctx.accounts.round_record;

    require!(
        record.status == RoundRecordStatus::Pending,
        MolphaError::RoundNotPending
    );
    // Challenger must supply the exact committed signature.
    require!(
        hash_agg_sig(&agg_sig) == record.agg_sig_hash,
        MolphaError::DisputeAggSigMismatch
    );
    require!(
        agg_sig.signers_bitmap == record.signers_bitmap,
        MolphaError::CreditedBitmapMismatch
    );

    let is_valid_sig = {
        let registry = ctx.accounts.registry.load()?;
        require!(
            registry.version == record.registry_version,
            MolphaError::InvalidRegistryVersion
        );
        let node_count = u32::from(registry.node_count);
        let expected_selection = molpha_verifier::derive_selection_bitmap(
            &record.source_id,
            record.registry_version,
            record.timestamp as u64,
            node_count,
            record.signatures_required,
            registry.redundancy_buffer,
        )
        .map_err(map_attestation_err)?;

        let selection_mismatch = !bitmap_is_subset(&record.signers_bitmap, &expected_selection);

        // Only run coalition reconstruction / Schnorr verification after deterministic
        // selection validation passes.
        if selection_mismatch {
            false
        } else {
            let signature = SchnorrSignature {
                agg_sig_s: agg_sig.s,
                commitment_addr: agg_sig.commitment_addr,
                signers_bitmap: agg_sig.signers_bitmap,
            };
            molpha_verifier::solana::verify_aggregate_over_hash_accounts(
                &ctx.accounts.registry.to_account_info(),
                signature,
                &record.attestation_message_hash,
                ctx.remaining_accounts,
                &ctx.program_id,
            )
            .map_err(map_attestation_accounts_err)?
        }
    };

    require!(!is_valid_sig, MolphaError::DisputeSignatureValid);

    // Undo payer-side settlement effects before slashing. x402 rounds are not refunded — see the
    // module doc.
    let round_restored = match ctx.accounts.round_record.kind {
        RoundRecordKind::X402 => {
            require!(
                ctx.accounts.subscription.is_none(),
                MolphaError::DisputeRefundAccountsMissing
            );
            verify_x402_record_pda(&ctx)?;
            false
        }
        RoundRecordKind::Subscription => {
            restore_subscription_round(&mut ctx)?;
            true
        }
    };

    // Fraud: void the round and slash the gateway bond from the gateway ATA.
    let config = &ctx.accounts.protocol_config;
    let slashed_bond = ctx.accounts.gateway.bond;
    require!(
        ctx.accounts.gateway_bond.amount >= slashed_bond,
        MolphaError::InsufficientBond
    );
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
    gateway.bond = 0;
    gateway.slashed = true;

    let record = &mut ctx.accounts.round_record;
    record.status = RoundRecordStatus::Voided;

    emit!(RoundDisputed {
        source_id: record.source_id,
        timestamp: record.timestamp,
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

/// Self-consistency check binding the supplied `round_record` to its x402 seeds. Possible now
/// that `payer_id` lives on the record itself (previously this required the now-deleted escrow
/// account). Defense-in-depth only — records can only be created by our own instructions.
fn verify_x402_record_pda<'info>(ctx: &Context<DisputeRound<'info>>) -> Result<()> {
    let record = &ctx.accounts.round_record;
    let expected_record = Pubkey::create_program_address(
        &[
            RoundReceiptRecord::X402_SEED_PREFIX,
            record.payer_id.as_ref(),
            record.gateway.as_ref(),
            record.source_id.as_ref(),
            &[record.signatures_required],
            &record.timestamp.to_le_bytes(),
            &[record.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        record.key(),
        expected_record,
        MolphaError::DisputeRefundAccountsMissing
    );
    Ok(())
}

fn restore_subscription_round<'info>(ctx: &mut Context<DisputeRound<'info>>) -> Result<()> {
    let subscription = ctx
        .accounts
        .subscription
        .as_mut()
        .ok_or_else(|| error!(MolphaError::DisputeRefundAccountsMissing))?;
    let expected_sub = Pubkey::create_program_address(
        &[
            Subscription::SEED_PREFIX,
            subscription.owner.as_ref(),
            &[subscription.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        subscription.key(),
        expected_sub,
        MolphaError::DisputeRefundAccountsMissing
    );

    // Round PDA is seeded with subscription.owner, so owner-direct and
    // delegate-settled rounds of the same subscription share one address.
    let owner = subscription.owner;
    let record = &ctx.accounts.round_record;
    let expected_record = Pubkey::create_program_address(
        &[
            RoundReceiptRecord::SEED_PREFIX,
            owner.as_ref(),
            record.source_id.as_ref(),
            &[record.signatures_required],
            &record.timestamp.to_le_bytes(),
            &[record.bump],
        ],
        ctx.program_id,
    )
    .map_err(|_| error!(MolphaError::DisputeRefundAccountsMissing))?;
    require_keys_eq!(
        ctx.accounts.round_record.key(),
        expected_record,
        MolphaError::DisputeRefundAccountsMissing
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

    /// Subscription or x402 settlement record. No seeds constraint — same rationale as
    /// `finalize_round` (owner + discriminator). PDA is re-derived in-handler against the
    /// relevant path (subscription restore, or x402 self-consistency check).
    #[account(mut)]
    pub round_record: Box<Account<'info, RoundReceiptRecord>>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Box<Account<'info, Gateway>>,

    #[account(
        mut,
        constraint = gateway_bond.mint == protocol_config.usdc_mint @ MolphaError::InvalidOwnerUsdc,
        constraint = gateway_bond.owner == gateway.key() @ MolphaError::InvalidOwnerUsdc,
    )]
    pub gateway_bond: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = challenger_usdc.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = challenger_usdc.owner == challenger.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub challenger_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,

    /// Subscription path (`record.kind == Subscription`). Must be `None` for x402 rounds.
    #[account(mut)]
    pub subscription: Option<Account<'info, Subscription>>,
}
