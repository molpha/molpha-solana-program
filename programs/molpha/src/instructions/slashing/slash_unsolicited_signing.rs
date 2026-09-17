//! Valid attestation with a signer outside the duty's deterministic selection set.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::Attestation;
use molpha_verifier::{bitmap::Bitmap, derive_selection_bitmap, solana::resolve_signers_accounts};

use crate::{
    error::{map_attestation_accounts_err, MolphaError},
    events::UnsolicitedSigningProven,
    state::{
        unsolicited_signing_evidence_hash, OffenceKind, ProtocolConfig, PunishmentCase, Registry,
        RegistryState,
    },
    utils::slash::{slash_nodes, verify_evidence_signature, SlashAccounts, SlashInput},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishUnsolicitedSigningArgs {
    pub attestation: Attestation,
    /// Must match `unsolicited_signing_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
}

pub fn punish_unsolicited_signing<'info>(
    ctx: Context<'info, PunishUnsolicitedSigning<'info>>,
    args: PunishUnsolicitedSigningArgs,
) -> Result<()> {
    let attestation = &args.attestation;

    require!(
        unsolicited_signing_evidence_hash(attestation) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    let signers_bitmap = Bitmap::load(&attestation.signature.signers_bitmap);
    let selection = derive_selection_bitmap(
        &attestation.payload.source_id,
        attestation.payload.canonical_timestamp,
        attestation.payload.signatures_required,
        &ctx.accounts.evidence_registry.load()?.view(),
    )
    .map_err(|_| error!(MolphaError::InvalidSignersBitmap))?;
    require!(
        !signers_bitmap.is_subset(&selection),
        MolphaError::SignersFullySelected
    );

    let guilty_bitmap = signers_bitmap.subtract(&selection);

    // Resolve the full submitted bitmap; the aggregate is only verifiable over all contributors.
    // `molpha_verifier::verify` would reject this evidence outright, since it enforces the very
    // selection membership the offence breaks.
    let signers = resolve_signers_accounts(
        ctx.remaining_accounts,
        ctx.accounts.evidence_registry.as_ref(),
        &attestation.signature.signers_bitmap,
    )
    .map_err(map_attestation_accounts_err)?;
    verify_evidence_signature(attestation, &signers)?;

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
            signers_bitmap,
            guilty_bitmap,
            program_id: ctx.program_id,
            offence: OffenceKind::UnsolicitedSigning,
            registry_version: args.attestation.payload.registry_version,
            evidence_hash: args.evidence_hash,
        },
    )?;

    emit!(UnsolicitedSigningProven {
        source_id: args.attestation.payload.source_id,
        registry_version: args.attestation.payload.registry_version,
        signatures_required: args.attestation.payload.signatures_required,
        canonical_timestamp: args.attestation.payload.canonical_timestamp,
        signers_bitmap: args.attestation.signature.signers_bitmap,
        guilty_bitmap: guilty_bitmap.to_bytes(),
        evidence_hash: args.evidence_hash,
        reporter,
        slashed_total: outcome.slashed_total,
        bounty_paid: outcome.bounty_paid,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: PunishUnsolicitedSigningArgs)]
pub struct PunishUnsolicitedSigning<'info> {
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
        seeds = [Registry::SEED_PREFIX, args.attestation.payload.registry_version.to_le_bytes().as_ref()],
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
        seeds = [PunishmentCase::SEED_PREFIX, args.evidence_hash.as_ref()],
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
