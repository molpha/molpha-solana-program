//! Two valid attestations share a nonce commitment but sign different messages.
//!
//! Same-duty different-value pairs must use `punish_equivocation`. Cross-version reuse is out of scope.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::bitmap::{bitmap_bit_set, bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::NonceReuseProven,
    instructions::disputing::{statement_message_hash, verify_statement},
    state::{
        nonce_reuse_evidence_hash, resolve_registry_signers_indexed, OffenceKind, ProtocolConfig,
        PunishmentCase, Registry, RegistryState, SignedStatement, SigningContext,
    },
    utils::punish::{execute_punishment, ExecutePunishmentAccounts, ExecutePunishmentInput},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishNonceReuseArgs {
    pub registry_version: u32,
    pub context_a: SigningContext,
    pub statement_a: SignedStatement,
    pub context_b: SigningContext,
    pub statement_b: SignedStatement,
    /// Must match `nonce_reuse_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

pub fn punish_nonce_reuse<'info>(
    ctx: Context<'info, PunishNonceReuse<'info>>,
    args: PunishNonceReuseArgs,
) -> Result<()> {
    require!(
        args.statement_a.commitment_addr == args.statement_b.commitment_addr,
        MolphaError::NonceCommitmentMismatch
    );

    let message_hash_a = statement_message_hash(
        &args.context_a.source_id,
        args.registry_version,
        args.context_a.signatures_required,
        args.context_a.canonical_timestamp,
        &args.statement_a,
    );
    let message_hash_b = statement_message_hash(
        &args.context_b.source_id,
        args.registry_version,
        args.context_b.signatures_required,
        args.context_b.canonical_timestamp,
        &args.statement_b,
    );
    require!(
        message_hash_a != message_hash_b,
        MolphaError::NonceReuseSameMessage
    );

    require!(
        !(args.context_a == args.context_b && args.statement_a.value != args.statement_b.value),
        MolphaError::PreferEquivocation
    );

    require!(
        nonce_reuse_evidence_hash(
            args.registry_version,
            &args.context_a,
            &args.statement_a,
            &args.context_b,
            &args.statement_b,
        ) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

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

        for (context, statement) in [
            (&args.context_a, &args.statement_a),
            (&args.context_b, &args.statement_b),
        ] {
            let selection = molpha_verifier::derive_selection_bitmap(
                &context.source_id,
                args.registry_version,
                context.canonical_timestamp,
                node_count,
                context.signatures_required,
                evidence_registry.redundancy_buffer,
            )
            .map_err(map_attestation_err)?;
            require!(
                bitmap_is_subset(&statement.signers_bitmap, &selection),
                MolphaError::SignersNotSubsetOfSelection
            );
        }

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
        &args.context_a.source_id,
        args.registry_version,
        args.context_a.signatures_required,
        args.context_a.canonical_timestamp,
        &args.statement_a,
        &ordered_a,
    )?;
    verify_statement(
        &args.context_b.source_id,
        args.registry_version,
        args.context_b.signatures_required,
        args.context_b.canonical_timestamp,
        &args.statement_b,
        &ordered_b,
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
            offence: OffenceKind::NonceReuse,
            registry_version: args.registry_version,
            evidence_hash: args.evidence_hash,
            new_registry_version: args.new_registry_version,
        },
    )?;

    emit!(NonceReuseProven {
        registry_version: args.registry_version,
        commitment_addr: args.statement_a.commitment_addr,
        source_id_a: args.context_a.source_id,
        signatures_required_a: args.context_a.signatures_required,
        canonical_timestamp_a: args.context_a.canonical_timestamp,
        signers_bitmap_a: args.statement_a.signers_bitmap,
        source_id_b: args.context_b.source_id,
        signatures_required_b: args.context_b.signatures_required,
        canonical_timestamp_b: args.context_b.canonical_timestamp,
        signers_bitmap_b: args.statement_b.signers_bitmap,
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
#[instruction(args: PunishNonceReuseArgs)]
pub struct PunishNonceReuse<'info> {
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
