//! Valid attestation with a signer outside the duty's deterministic selection set.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::bitmap::{bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::UnsolicitedSigningProven,
    instructions::disputing::verify_statement,
    state::{
        resolve_registry_signers_indexed, unsolicited_signing_evidence_hash, OffenceKind,
        ProtocolConfig, PunishmentCase, Registry, RegistryState, SignedStatement,
    },
    utils::punish::{execute_punishment, ExecutePunishmentAccounts, ExecutePunishmentInput},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishUnsolicitedSigningArgs {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
    pub statement: SignedStatement,
    /// Must match `unsolicited_signing_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

pub fn punish_unsolicited_signing<'info>(
    ctx: Context<'info, PunishUnsolicitedSigning<'info>>,
    args: PunishUnsolicitedSigningArgs,
) -> Result<()> {
    require!(
        unsolicited_signing_evidence_hash(
            &args.source_id,
            args.registry_version,
            args.signatures_required,
            args.canonical_timestamp,
            &args.statement,
        ) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

    // Resolve the full submitted bitmap; the aggregate is only verifiable over all contributors.
    let (resolved, guilty_bitmap, guilty_count) = {
        let evidence_registry = ctx.accounts.evidence_registry.load()?;

        let node_count = u32::from(evidence_registry.node_count);
        let selection = molpha_verifier::derive_selection_bitmap(
            &args.source_id,
            args.registry_version,
            args.canonical_timestamp,
            node_count,
            args.signatures_required,
            evidence_registry.redundancy_buffer,
        )
        .map_err(map_attestation_err)?;
        require!(
            !bitmap_is_subset(&args.statement.signers_bitmap, &selection),
            MolphaError::SignersFullySelected
        );
        let guilty = bitmap_load(&args.statement.signers_bitmap) & !bitmap_load(&selection);
        let guilty_bitmap = bitmap_store(guilty);
        let guilty_count = guilty.count_ones() as u16;

        let resolved = resolve_registry_signers_indexed(
            ctx.remaining_accounts,
            ctx.program_id,
            &evidence_registry,
            &args.statement.signers_bitmap,
        )?;
        (resolved, guilty_bitmap, guilty_count)
    };
    let ordered: Vec<molpha_verifier::SignerXy> = resolved.iter().map(|(_, xy)| *xy).collect();

    verify_statement(
        &args.source_id,
        args.registry_version,
        args.signatures_required,
        args.canonical_timestamp,
        &args.statement,
        &ordered,
    )?;

    let reporter = ctx.accounts.reporter.key();
    let outcome = execute_punishment(
        ExecutePunishmentAccounts {
            registry_state: &mut ctx.accounts.registry_state,
            current_registry: &ctx.accounts.current_registry,
            new_registry: &ctx.accounts.new_registry,
            new_registry_bump: ctx.bumps.new_registry,
            protocol_config: &ctx.accounts.protocol_config,
            punishment_case: &mut ctx.accounts.punishment_case,
            punishment_case_bump: ctx.bumps.punishment_case,
            reporter,
            treasury: &ctx.accounts.treasury,
            reporter_usdc: &ctx.accounts.reporter_usdc,
            token_program: ctx.accounts.token_program.key(),
        },
        ExecutePunishmentInput {
            remaining_accounts: ctx.remaining_accounts,
            program_id: ctx.program_id,
            resolved: &resolved,
            guilty_bitmap,
            guilty_count,
            offence: OffenceKind::UnsolicitedSigning,
            registry_version: args.registry_version,
            evidence_hash: args.evidence_hash,
            new_registry_version: args.new_registry_version,
        },
    )?;

    emit!(UnsolicitedSigningProven {
        source_id: args.source_id,
        registry_version: args.registry_version,
        signatures_required: args.signatures_required,
        canonical_timestamp: args.canonical_timestamp,
        signers_bitmap: args.statement.signers_bitmap,
        guilty_bitmap,
        guilty_count,
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
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub evidence_registry: AccountLoader<'info, Registry>,

    #[account(
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = reporter,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, args.new_registry_version.to_le_bytes().as_ref()],
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
        constraint = treasury.key() == protocol_config.treasury @ MolphaError::InvalidTreasury,
        constraint = treasury.owner == protocol_config.key() @ MolphaError::InvalidTreasuryOwner,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = reporter_usdc.mint == protocol_config.usdc_mint @ MolphaError::InvalidWithdrawDestination,
        constraint = reporter_usdc.owner == reporter.key() @ MolphaError::InvalidWithdrawDestination,
    )]
    pub reporter_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
