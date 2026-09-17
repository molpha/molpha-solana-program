//! Two valid attestations for the same duty with different values. Guilty set = signer intersection.
//!
//! Cross-version equivocation is out of scope. Gateways must use fresh timestamps on retry.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::{
    solana::resolve_intersected_signers_accounts
};

use crate::{
    error::MolphaError,
    events::EquivocationProven,
    state::{
        EquivocationEvidence, ProtocolConfig, PunishmentCase, Registry,
        RegistryState, Evidence,
    },
    utils::slash::{is_equivocation, slash_nodes, SlashAccounts, SlashInput, verify_evidence_signature},
};

pub fn slash_equivocation<'info>(
    ctx: Context<'info, SlashEquivocation<'info>>,
    evidence: EquivocationEvidence,
) -> Result<()> {
    require!(
        is_equivocation(&evidence.attestation_a, &evidence.attestation_b),
        MolphaError::NotEquivocation
    );

    let resolution = resolve_intersected_signers_accounts(
        ctx.remaining_accounts,
        &ctx.accounts.evidence_registry.as_ref(),
        &evidence.attestation_a.signature.signers_bitmap,
        &evidence.attestation_b.signature.signers_bitmap,
    )
    .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
    require!(
        resolution.intersected_bitmap.popcount() > 0,
        MolphaError::GuiltySetEmpty
    );
    verify_evidence_signature(&evidence.attestation_a, &resolution.signers_a)?;
    verify_evidence_signature(&evidence.attestation_b, &resolution.signers_b)?;

    let reporter = ctx.accounts.reporter.key();
    let outcome = slash_nodes(
        SlashAccounts {
            registry_state: &mut ctx.accounts.registry_state,
            current_registry: &ctx.accounts.current_registry,
            new_registry: &ctx.accounts.new_registry,
            new_registry_bump: ctx.bumps.new_registry,
            protocol_config: &mut ctx.accounts.protocol_config,
            punishment_case: &mut ctx.accounts.punishment_case,
            punishment_case_bump: ctx.bumps.punishment_case,
            reporter,
            node_deposit_vault: &mut ctx.accounts.node_deposit_vault,
            treasury: &mut ctx.accounts.treasury,
            reporter_usdc: &ctx.accounts.reporter_usdc,
            token_program: ctx.accounts.token_program.key(),
        },
        SlashInput {
            signer_accounts: ctx.remaining_accounts,
            signers_bitmap: resolution.unioned_bitmap,
            guilty_bitmap: resolution.intersected_bitmap,
            program_id: ctx.program_id,
            evidence: &evidence,
        },
    )?;

    emit!(EquivocationProven {
        source_id: evidence.attestation_a.payload.source_id,
        registry_version: evidence.registry_version(),
        signatures_required: evidence.attestation_a.payload.signatures_required,
        canonical_timestamp: evidence.attestation_a.payload.canonical_timestamp,
        guilty_bitmap: resolution.intersected_bitmap.to_bytes(),
        evidence_hash: evidence.evidence_hash(),
        reporter,
        slashed_total: outcome.slashed_total,
        bounty_paid: outcome.bounty_paid,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(evidence: EquivocationEvidence)]
pub struct SlashEquivocation<'info> {
    #[account(mut)]
    pub reporter: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, evidence.registry_version().to_le_bytes().as_ref()],
        bump,
    )]
    pub evidence_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = reporter,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = reporter,
        space = PunishmentCase::SPACE,
        seeds = [PunishmentCase::SEED_PREFIX, evidence.evidence_hash().as_ref()],
        bump,
    )]
    pub punishment_case: Box<Account<'info, PunishmentCase>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    /// Must already exist. Creating it here would cost two more transaction slots, which the
    /// signer list needs more than a reporter needs the convenience.
    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = reporter,
        associated_token::token_program = token_program,
    )]
    pub reporter_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
