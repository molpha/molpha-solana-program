//! `punish_nonce_reuse` — offence #3: a node contributes a partial signature under one aggregate
//! nonce commitment for two different signed messages.
//!
//! Permissionless and immediate: the proof is two valid Molpha attestations
//! (`statement_a` under `context_a`, `statement_b` under `context_b`) sharing an aggregate
//! nonce commitment (`SignedStatement::commitment_addr`) but committing to different message
//! hashes. Because the verifier's challenge is
//! `e = keccak(P.x || parity || msg_hash || commitment_addr) mod n`
//! (`molpha_verifier::scalar`), and `commitment_addr` is a function of the nonce alone, a node
//! cannot compute its partial share without first knowing which commitment it is signing under —
//! refusing a second, different message under a commitment it has already used is a purely local,
//! always-checkable rule, so this is a genuine protocol violation and never a frame-up. Because
//! `compute_message_hash` commits to `signers_bitmap`, two statements sharing a commitment with
//! different message hashes necessarily have `e_a != e_b`; that inequality is exactly this
//! offence's predicate, and (since `(commitment_addr, msg_hash)` pins `s` uniquely) it also rules
//! out the pair being one statement submitted twice.
//!
//! Both statements must be signed against the same `registry_version` — cross-version nonce reuse
//! is out of scope, mirroring `punish_equivocation`'s own cross-version limitation, and is
//! similarly self-limiting since selection is version-derived. The guilty set is the intersection
//! of the two signer bitmaps, exactly as with equivocation: a node outside the intersection signed
//! under this commitment only once, and no local check would have fired for it.
//!
//! A statement pair that also shares one full duty tuple (`context_a == context_b`) but commits to
//! *different* `value`s is an equivocation, not nonce reuse, and must be filed as
//! `punish_equivocation` for its full 100% sentence — see [`MolphaError::PreferEquivocation`].
//! Without that guard a coalition could self-report the cheaper 50% case first, tombstoning itself
//! and shielding the other half via [`MolphaError::NothingToPunish`] on a later equivocation
//! filing (`punish_guilty_nodes` skips already-tombstoned nodes). The same reasoning is why the
//! selection-subset check below is kept for both statements: a signer outside its own selection is
//! unsolicited signing, also 100%, and must not be launderable into this offence either.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use molpha_verifier::bitmap::{bitmap_bit_set, bitmap_is_subset, bitmap_load, bitmap_store};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::{NonceReuseProven, RegistryVersionCreated, RegistryVersionReason},
    instructions::disputing::{statement_message_hash, verify_statement},
    state::{
        nonce_reuse_evidence_hash, resolve_registry_signers_indexed, OffenceKind, ProtocolConfig,
        PunishmentCase, Registry, RegistryState, SignedStatement, SigningContext,
    },
    utils::punish::punish_guilty_nodes,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishNonceReuseArgs {
    /// Registry version both statements were signed against; also the version `guilty_bitmap` on
    /// `PunishmentCase` indexes into.
    pub registry_version: u32,
    pub context_a: SigningContext,
    pub statement_a: SignedStatement,
    pub context_b: SigningContext,
    pub statement_b: SignedStatement,
    /// Must equal `nonce_reuse_evidence_hash(..)` over the fields above — validated in the
    /// handler and reused as the `PunishmentCase` PDA seed (the anti-replay guard).
    pub evidence_hash: [u8; 32],
    pub new_registry_version: u32,
}

pub fn punish_nonce_reuse<'info>(
    ctx: Context<PunishNonceReuse<'info>>,
    args: PunishNonceReuseArgs,
) -> Result<()> {
    // ---- 1. Args-only predicates, cheapest first. ----
    require!(
        args.statement_a.commitment_addr == args.statement_b.commitment_addr,
        MolphaError::NonceCommitmentMismatch
    );

    // Distinct signed messages. Since `compute_message_hash` commits to `signers_bitmap`, equal
    // message hashes imply equal coalitions, so this inequality is exactly `e_a != e_b` — the
    // predicate the offence needs. It also subsumes an identical-evidence guard: with one
    // commitment and one message hash, `s` is uniquely determined, so two statements passing this
    // check can never be the same statement submitted twice.
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

    // Route the harsher offence to its own instruction — see the module doc.
    require!(
        !(args.context_a == args.context_b && args.statement_a.value != args.statement_b.value),
        MolphaError::PreferEquivocation
    );

    // ---- 2. Evidence hash doubles as the PunishmentCase PDA seed / replay guard. ----
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

    // ---- 4. Guilty set = intersection of the two signer bitmaps, mirroring equivocation. ----
    let bitmap_a = bitmap_load(&args.statement_a.signers_bitmap);
    let bitmap_b = bitmap_load(&args.statement_b.signers_bitmap);
    let guilty = bitmap_a & bitmap_b;
    require!(guilty.count_ones() > 0, MolphaError::GuiltySetEmpty);
    let guilty_bitmap = bitmap_store(guilty);
    let guilty_count = guilty.count_ones() as u16;
    let union_bitmap = bitmap_store(bitmap_a | bitmap_b);

    // ---- 5. One pinned evidence registry, one union resolution; per-statement selection derived
    // separately since source_id / canonical_timestamp / signatures_required may all differ
    // between the two contexts. The subset check is kept for both statements for the same reason
    // as the `PreferEquivocation` guard above — see the module doc. ----
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

    // ---- 6. Re-partition the resolved union back into each statement's ordered signers. ----
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

    // ---- 7. Both aggregates must be valid — that is what proves every intersection member
    // actually contributed a partial share under this commitment. ----
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

    // ---- Sentence: 50% slash + tombstone every guilty node, evict from a fresh registry
    // snapshot, pay the reporter a bounty out of the total slashed. ----
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let now_slot = clock.slot;
    let program_id = ctx.program_id;

    let batch = punish_guilty_nodes(
        ctx.remaining_accounts,
        &resolved,
        &guilty_bitmap,
        OffenceKind::NonceReuse,
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
        OffenceKind::NonceReuse,
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
