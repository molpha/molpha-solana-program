//! `punish_unsolicited_signing` — offence #2: a node's signature appears in a valid Molpha
//! attestation whose signer bitmap contains a bit outside the duty's deterministic selection set.
//!
//! Permissionless and immediate: the proof is a single valid Molpha attestation
//! (`source_id`, `registry_version`, `signatures_required`, `canonical_timestamp`, `value`) whose
//! `signers_bitmap` is not a subset of `derive_selection_bitmap(..)` for that duty. Because the
//! coalition key is a plain sum `Σ X_i` protected by proof-of-possession at registration
//! (`utils::registry::verify_node_pop`), and `compute_message_hash` commits to `signers_bitmap`
//! itself, a valid aggregate signature over a bitmap is only producible with the secret behind
//! every node in it — so a set bit outside the selection demonstrably contributed a partial
//! signature to a duty it was never assigned. The selection is a pure function of fields the node
//! itself signed over, so it could always have checked and refused; an innocent node can never be
//! framed this way, since framing needs its secret key.
//!
//! The guilty set is exactly `signers_bitmap & !selection` — legitimately-selected cosigners in
//! the same coalition are untouched, mirroring how `punish_equivocation` punishes only the
//! intersection of two signer sets. The sentence is a 100% slash and a permanent tombstone
//! (`OffenceKind::UnsolicitedSigning::penalty`).
//!
//! This is complementary to, and independent of, `dispute_round`: that instruction slashes the
//! *gateway* that assembled and settled a round outside the selection (and only within the dispute
//! window, from an on-chain `RoundReceiptRecord`); this instruction slashes the *node(s)* that
//! signed it, permissionlessly, from self-contained evidence, with no dispute window and no
//! dependency on the round ever having been settled on-chain.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::bitmap::{bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_data_update_err, MolphaError},
    events::{RegistryVersionCreated, RegistryVersionReason, UnsolicitedSigningProven},
    instructions::disputing::verify_statement,
    state::{
        resolve_registry_signers_indexed, unsolicited_signing_evidence_hash, OffenceKind,
        ProtocolConfig, PunishmentCase, Registry, RegistryState, SignedStatement,
    },
    utils::punish::punish_guilty_nodes,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishUnsolicitedSigningArgs {
    pub source_id: [u8; 32],
    /// Registry version the statement was signed against; also the version `guilty_bitmap` on
    /// `PunishmentCase` indexes into.
    pub registry_version: u32,
    pub signatures_required: u8,
    pub canonical_timestamp: i64,
    pub statement: SignedStatement,
    /// Must equal `unsolicited_signing_evidence_hash(..)` over the fields above — validated in the
    /// handler and reused as the `PunishmentCase` PDA seed (the anti-replay guard).
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

pub fn punish_unsolicited_signing<'info>(
    ctx: Context<PunishUnsolicitedSigning<'info>>,
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

    // ---- The guilty set is exactly the submitted bitmap minus the derived selection. Resolve
    // the full submitted bitmap (not just the guilty bits) — the aggregate signature is only
    // meaningful, and only checkable, over every signer that contributed to it. ----
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
        .map_err(map_data_update_err)?;
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
    let ordered: Vec<molpha_verifier::SignerXy> =
        resolved.iter().map(|(_, xy)| *xy).collect();

    verify_statement(
        &args.source_id,
        args.registry_version,
        args.signatures_required,
        args.canonical_timestamp,
        &args.statement,
        &ordered,
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
        OffenceKind::UnsolicitedSigning,
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
        OffenceKind::UnsolicitedSigning,
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

    emit!(UnsolicitedSigningProven {
        source_id: args.source_id,
        registry_version: args.registry_version,
        signatures_required: args.signatures_required,
        canonical_timestamp: args.canonical_timestamp,
        signers_bitmap: args.statement.signers_bitmap,
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

    /// The registry snapshot the statement was signed against.
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
