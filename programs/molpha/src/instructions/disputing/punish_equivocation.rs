//! `punish_equivocation` — offence #1: a node signs two different statements for the same
//! execution duty.
//!
//! Permissionless and immediate: the proof is two valid Molpha attestations sharing a duty
//! identity (`source_id`, `registry_version`, `signatures_required`, `canonical_timestamp`) but
//! committing to different `value`s. Because the coalition key is a plain sum `Σ X_i` protected
//! by proof-of-possession at registration (`utils::registry::verify_node_pop`), a valid aggregate
//! over a bitmap is only producible with the secret behind every node in it — so a node in the
//! intersection of the two signer bitmaps demonstrably contributed a partial signature to both.
//! The guilty set is exactly that intersection; the sentence is a 100% slash and a permanent
//! tombstone (`OffenceKind::Equivocation::penalty`).
//!
//! Known limitation: an equivocation split across two registry versions (same
//! `source_id`/`canonical_timestamp`, different `registry_version`) is out of scope. It is
//! largely self-limiting since the selection set is version-derived, so a node is unlikely to be
//! selected in both.
//!
//! This is the whole on-chain safety argument, so two off-chain rules become load-bearing: a
//! node must never sign two different values for the same duty tuple (cache and replay the first
//! answer), and a gateway retrying a failed round must mint a fresh `canonical_timestamp` rather
//! than reusing one — exactly the monotonicity `submit_attestation` already enforces on the push
//! path (`FeedNotNewer`).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::bitmap::{bitmap_bit_set, bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::{EquivocationProven, RegistryVersionCreated, RegistryVersionReason},
    instructions::disputing::verify_statement,
    state::{
        equivocation_evidence_hash, resolve_registry_signers_indexed, OffenceKind, ProtocolConfig,
        PunishmentCase, Registry, RegistryState, SignedStatement,
    },
    utils::punish::punish_guilty_nodes,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishEquivocationArgs {
    pub source_id: [u8; 32],
    /// Registry version the two attestations were signed against; also the version
    /// `guilty_bitmap` on `PunishmentCase` indexes into.
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
    pub statement_a: SignedStatement,
    pub statement_b: SignedStatement,
    /// Must equal `equivocation_evidence_hash(..)` over the fields above — validated in the
    /// handler and reused as the `PunishmentCase` PDA seed (the anti-replay guard), mirroring how
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

pub fn punish_equivocation<'info>(
    ctx: Context<PunishEquivocation<'info>>,
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

    ctx.accounts
        .registry_state
        .require_next_version(args.new_registry_version)?;

    // ---- Resolve the union of both signer sets once, against the pinned evidence-version
    // snapshot, then re-partition into each statement's ordered signers. ----
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

    // ---- Sentence: full slash + tombstone every guilty node, evict from a fresh registry
    // snapshot, pay the reporter a bounty out of the total slashed. ----
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let now_slot = clock.slot;
    let program_id = ctx.program_id;

    let batch = punish_guilty_nodes(
        ctx.remaining_accounts,
        &resolved,
        &guilty_bitmap,
        OffenceKind::Equivocation,
        now,
        program_id,
    )?;
    require!(batch.punished_count > 0, MolphaError::NothingToPunish);
    let slashed_total = batch.slashed_total;

    let previous_version = ctx.accounts.registry_state.current_version;
    let (node_count_after, redundancy_buffer_after) = {
        let current = ctx.accounts.current_registry.load()?;
        let mut next = ctx.accounts.new_registry.load_init()?;
        crate::utils::punish::evict_nodes(
            &current,
            &mut next,
            args.new_registry_version,
            ctx.bumps.new_registry,
            &batch.victims,
        )?;
        (next.node_count, next.redundancy_buffer)
    };
    ctx.accounts.registry_state.current_version = args.new_registry_version;

    let config = &ctx.accounts.protocol_config;
    let bounty = (slashed_total as u128)
        .saturating_mul(config.challenger_bounty_bps as u128)
        .checked_div(10_000)
        .unwrap_or(0) as u64;
    if bounty > 0 {
        let bump = [config.bump];
        let signer_seeds: &[&[u8]] = &[ProtocolConfig::SEED_PREFIX, &bump];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.treasury.to_account_info(),
                    to: ctx.accounts.reporter_usdc.to_account_info(),
                    authority: ctx.accounts.protocol_config.to_account_info(),
                },
                &[signer_seeds],
            ),
            bounty,
        )?;
    }

    ctx.accounts.punishment_case.init(
        OffenceKind::Equivocation,
        ctx.accounts.reporter.key(),
        args.registry_version,
        guilty_bitmap,
        guilty_count,
        slashed_total,
        bounty,
        args.evidence_hash,
        now_slot,
        ctx.bumps.punishment_case,
    );

    emit!(EquivocationProven {
        source_id: args.source_id,
        registry_version: args.registry_version,
        signatures_required: args.signatures_required,
        canonical_timestamp: args.canonical_timestamp,
        guilty_bitmap,
        guilty_count,
        evidence_hash: args.evidence_hash,
        reporter: ctx.accounts.reporter.key(),
        slashed_total,
        bounty_paid: bounty,
    });
    emit!(RegistryVersionCreated {
        previous_version,
        new_version: args.new_registry_version,
        node_count: node_count_after,
        redundancy_buffer: redundancy_buffer_after,
        reason: RegistryVersionReason::NodePunished,
        created_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: PunishEquivocationArgs)]
pub struct PunishEquivocation<'info> {
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

    /// The registry snapshot both statements were signed against.
    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub evidence_registry: AccountLoader<'info, Registry>,

    /// The live registry snapshot guilty nodes are evicted from. May equal `evidence_registry`.
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
