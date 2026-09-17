//! Two valid attestations share a nonce commitment but sign different messages.
//!
//! Same-duty different-value pairs must use `punish_equivocation`. Cross-version reuse is out of scope.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::solana::resolve_intersected_signers_accounts;
use molpha_verifier::Attestation;

use crate::{
    error::MolphaError,
    events::CommitmentReuseProven,
    state::{
        commitment_reuse_evidence_hash, OffenceKind, ProtocolConfig, PunishmentCase, Registry,
        RegistryState,
    },
    utils::slash::{
        is_equivocation, require_signers_selected, slash_nodes, verify_evidence_signature,
        SlashAccounts, SlashInput,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishCommitmentReuseArgs {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
    /// Must match `commitment_reuse_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
}

pub fn punish_commitment_reuse<'info>(
    ctx: Context<'info, PunishCommitmentReuse<'info>>,
    args: PunishCommitmentReuseArgs,
) -> Result<()> {
    let (attestation_a, attestation_b) = (&args.attestation_a, &args.attestation_b);

    require!(
        attestation_a.signature.commitment == attestation_b.signature.commitment,
        MolphaError::NonceCommitmentMismatch
    );

    require!(
        attestation_a.payload.registry_version == attestation_b.payload.registry_version,
        MolphaError::RegistryVersionMismatch
    );

    // The signed message is the payload plus the signer bitmap. Reusing `R` for one message is
    // not an offence, and without this check any published attestation filed twice would slash
    // its own honest signers.
    require!(
        attestation_a.payload != attestation_b.payload
            || attestation_a.signature.signers_bitmap != attestation_b.signature.signers_bitmap,
        MolphaError::NonceReuseSameMessage
    );

    require!(
        !is_equivocation(attestation_a, attestation_b),
        MolphaError::PreferEquivocation
    );

    require!(
        commitment_reuse_evidence_hash(attestation_a, attestation_b) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    {
        let evidence_registry = ctx.accounts.evidence_registry.load()?;
        let view = evidence_registry.view();
        require_signers_selected(attestation_a, &view)?;
        require_signers_selected(attestation_b, &view)?;
    }

    let resolution = resolve_intersected_signers_accounts(
        ctx.remaining_accounts,
        ctx.accounts.evidence_registry.as_ref(),
        &attestation_a.signature.signers_bitmap,
        &attestation_b.signature.signers_bitmap,
    )
    .map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
    let guilty_bitmap = resolution.intersected_bitmap;
    require!(guilty_bitmap.popcount() > 0, MolphaError::GuiltySetEmpty);

    verify_evidence_signature(&args.attestation_a, &resolution.signers_a)?;
    verify_evidence_signature(&args.attestation_b, &resolution.signers_b)?;

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
            guilty_bitmap,
            program_id: ctx.program_id,
            offence: OffenceKind::CommitmentReuse,
            registry_version: args.attestation_a.payload.registry_version,
            evidence_hash: args.evidence_hash,
        },
    )?;

    emit!(CommitmentReuseProven {
        registry_version: args.attestation_a.payload.registry_version,
        commitment_addr: args.attestation_a.signature.commitment,
        source_id_a: args.attestation_a.payload.source_id,
        signatures_required_a: args.attestation_a.payload.signatures_required,
        canonical_timestamp_a: args.attestation_a.payload.canonical_timestamp,
        signers_bitmap_a: args.attestation_a.signature.signers_bitmap,
        source_id_b: args.attestation_b.payload.source_id,
        signatures_required_b: args.attestation_b.payload.signatures_required,
        canonical_timestamp_b: args.attestation_b.payload.canonical_timestamp,
        signers_bitmap_b: args.attestation_b.signature.signers_bitmap,
        guilty_bitmap: guilty_bitmap.to_bytes(),
        evidence_hash: args.evidence_hash,
        reporter,
        slashed_total: outcome.slashed_total,
        bounty_paid: outcome.bounty_paid,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(args: PunishCommitmentReuseArgs)]
pub struct PunishCommitmentReuse<'info> {
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
        seeds = [Registry::SEED_PREFIX, args.attestation_a.payload.registry_version.to_le_bytes().as_ref()],
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
