//! Two valid attestations for the same duty with different values. Guilty set = signer intersection.
//!
//! Cross-version equivocation is out of scope. Gateways must use fresh timestamps on retry.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::bitmap::{bitmap_bit_set, bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::EquivocationProven,
    state::{
        equivocation_evidence_hash, resolve_registry_signers_indexed, OffenceKind, ProtocolConfig,
        PunishmentCase, Registry, RegistryState, SignedStatement,
    },
    utils::{
        slash::{slash_nodes, SlashAccounts, SlashInput},
        verify_statement,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishEquivocationArgs {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
    pub statement_a: SignedStatement,
    pub statement_b: SignedStatement,
    /// Must match `equivocation_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
}

pub fn punish_equivocation<'info>(
    ctx: Context<'info, PunishEquivocation<'info>>,
    args: PunishEquivocationArgs,
) -> Result<()> {
    require!(
        args.statement_a.value != args.statement_b.value,
        MolphaError::EquivocationSameStatement
    );
    require!(
        equivocation_evidence_hash(
            &args.source_id,
            args.registry_version,
            args.signatures_required,
            args.canonical_timestamp,
            &args.statement_a,
            &args.statement_b,
        ) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    let bitmap_a = bitmap_load(&args.statement_a.signers_bitmap);
    let bitmap_b = bitmap_load(&args.statement_b.signers_bitmap);
    let guilty = bitmap_a & bitmap_b;
    require!(guilty.count_ones() > 0, MolphaError::GuiltySetEmpty);
    let guilty_bitmap = bitmap_store(guilty);
    let guilty_count = guilty.count_ones() as u16;
    let union_bitmap = bitmap_store(bitmap_a | bitmap_b);

    let resolved = {
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
            bitmap_is_subset(&args.statement_a.signers_bitmap, &selection),
            MolphaError::SignersNotSubsetOfSelection
        );
        require!(
            bitmap_is_subset(&args.statement_b.signers_bitmap, &selection),
            MolphaError::SignersNotSubsetOfSelection
        );

        resolve_registry_signers_indexed(
            ctx.remaining_accounts,
            ctx.program_id,
            &evidence_registry,
            &union_bitmap,
        )?
    };

    let mut ordered_a = Vec::new();
    let mut ordered_b = Vec::new();
    for (bit_pos, xy) in &resolved {
        if bitmap_bit_set(&args.statement_a.signers_bitmap, *bit_pos) {
            ordered_a.push(*xy);
        }
        if bitmap_bit_set(&args.statement_b.signers_bitmap, *bit_pos) {
            ordered_b.push(*xy);
        }
    }

    verify_statement(
        &args.source_id,
        args.registry_version,
        args.signatures_required,
        args.canonical_timestamp,
        &args.statement_a,
        &ordered_a,
    )?;
    verify_statement(
        &args.source_id,
        args.registry_version,
        args.signatures_required,
        args.canonical_timestamp,
        &args.statement_b,
        &ordered_b,
    )?;

    let reporter = ctx.accounts.reporter.key();
    let new_registry_version = ctx.accounts.registry_state.next_version;
    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();

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
            program_id: ctx.program_id,
            resolved: &resolved,
            guilty_bitmap,
            guilty_count,
            offence: OffenceKind::Equivocation,
            registry_version: args.registry_version,
            evidence_hash: args.evidence_hash,
            new_registry_version,
            grace_active_until,
        },
    )?;

    emit!(EquivocationProven {
        source_id: args.source_id,
        registry_version: args.registry_version,
        signatures_required: args.signatures_required,
        canonical_timestamp: args.canonical_timestamp,
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
#[instruction(args: PunishEquivocationArgs)]
pub struct PunishEquivocation<'info> {
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
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
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
