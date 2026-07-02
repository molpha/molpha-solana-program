//! `dispute_round` — the only place Schnorr verification runs.
//!
//! A challenger supplies the committed `AggregatedSig` (preimage of `record.agg_sig_hash`) and
//! forces verification against the registry keys selected by the signature's own
//! `signers_bitmap`. Because the credited set *is* that bitmap and the scheme is a plain-sum
//! coalition key, an over-credited (inflated) set cannot produce a valid aggregate (§1.1).
//!
//! - Invalid selection bitmap, or invalid signature over the credited set
//!   (fabricated round / committed garbage) -> void the round and slash `gateway.bond`,
//!   paying the challenger `challenger_bounty_bps`.
//! - Valid selection bitmap and valid signature -> no fraud; dispute fails
//!   (`DisputeSignatureValid`).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{
    error::{map_data_update_err, MolphaError},
    events::RoundDisputed,
    state::{
        collect_node_entries, hash_agg_sig, AggregatedSig, Gateway, ProtocolConfig, RegistryState,
        RoundReceiptRecord,
    },
    utils::bitmap::bitmap_is_subset,
};

pub fn dispute_round<'info>(
    ctx: Context<'_, '_, '_, 'info, DisputeRound<'info>>,
    agg_sig: AggregatedSig,
) -> Result<()> {
    let record = &ctx.accounts.round_record;

    require!(!record.voided, MolphaError::RoundVoided);
    // Disputes close once the round is finalized (the dispute window has passed).
    require!(!record.finalized, MolphaError::DisputeWindowClosed);

    // Challenger must supply the exact committed signature.
    require!(
        hash_agg_sig(&agg_sig) == record.agg_sig_hash,
        MolphaError::DisputeAggSigMismatch
    );
    require!(
        agg_sig.signers_bitmap == record.signers_bitmap,
        MolphaError::CreditedBitmapMismatch
    );

    let clock = Clock::get()?;
    let is_current = record.registry_version == ctx.accounts.registry_state.current_version;
    let is_previous_live = record.registry_version == ctx.accounts.registry_state.previous_version
        && clock.unix_timestamp <= ctx.accounts.registry_state.previous_expires_at;
    require!(
        is_current || is_previous_live,
        MolphaError::InvalidRegistryVersion
    );
    let node_count = if is_current {
        ctx.accounts.registry_state.current_node_count
    } else {
        ctx.accounts.registry_state.previous_node_count
    };
    let expected_selection = molpha_verifier::derive_selection_bitmap(
        &record.job_id,
        record.registry_version,
        record.timestamp as i64,
        node_count,
        record.signatures_required,
        ctx.accounts.registry_state.redundancy_buffer,
    )
    .map_err(map_data_update_err)?;
    let selection_mismatch = bitmap_is_subset(&expected_selection, &record.signers_bitmap);

    // Only run coalition reconstruction / Schnorr verification after deterministic
    // selection validation passes.
    let is_valid_sig = if selection_mismatch {
        false
    } else {
        let entries = collect_node_entries(ctx.remaining_accounts, ctx.program_id)?;
        molpha_verifier::verify_aggregate_over_hash_resolved(
            &ctx.accounts.registry_state.view(),
            record.registry_version,
            &agg_sig.signers_bitmap,
            &agg_sig.s,
            &agg_sig.commitment_addr,
            &record.data_update_message_hash,
            clock.unix_timestamp,
            &entries,
        )
        .map_err(map_data_update_err)?
    };

    // No fraud iff both deterministic selection and aggregate signature are valid.
    require!(
        selection_mismatch || !is_valid_sig,
        MolphaError::DisputeSignatureValid
    );

    // Fraud: void the round and slash the gateway bond.
    let config = &ctx.accounts.protocol_config;
    let slashed_bond = ctx.accounts.gateway.bond;
    let bounty = (slashed_bond as u128)
        .saturating_mul(config.challenger_bounty_bps as u128)
        .checked_div(10_000)
        .unwrap_or(0) as u64;

    if bounty > 0 {
        let bump = [config.bump];
        let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.treasury.to_account_info(),
                    to: ctx.accounts.challenger_usdc.to_account_info(),
                    authority: ctx.accounts.protocol_config.to_account_info(),
                },
                &[signer_seeds],
            ),
            bounty,
        )?;
    }

    let gateway = &mut ctx.accounts.gateway;
    gateway.bond = 0;
    gateway.slashed = true;

    let record = &mut ctx.accounts.round_record;
    record.voided = true;

    emit!(RoundDisputed {
        job_id: record.job_id,
        timestamp: record.timestamp,
        gateway: record.gateway,
        slashed_bond,
        challenger_bounty: bounty,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct DisputeRound<'info> {
    #[account(mut)]
    pub challenger: Signer<'info>,

    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Account<'info, RegistryState>,

    #[account(
        mut,
        seeds = [
            RoundReceiptRecord::SEED_PREFIX,
            round_record.job_id.as_ref(),
            round_record.timestamp.to_le_bytes().as_ref(),
        ],
        bump = round_record.bump,
    )]
    pub round_record: Account<'info, RoundReceiptRecord>,

    #[account(
        mut,
        constraint = gateway.key() == round_record.gateway @ MolphaError::GatewayIndexMismatch,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        mut,
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = challenger_usdc.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = challenger_usdc.owner == challenger.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub challenger_usdc: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
