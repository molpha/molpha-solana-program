//! Two valid attestations for the same duty with different values. Guilty set = signer intersection.
//!
//! Cross-version equivocation is out of scope. Gateways must use fresh timestamps on retry.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::solana::{resolve_intersected_signers_accounts};


use crate::{
    error::MolphaError,
    events::EquivocationProven,
    state::{
        equivocation_evidence_hash, OffenceKind, ProtocolConfig,
        PunishmentCase, Registry, RegistryState, SignedStatement,
    },
    utils::{
        settlement::RoundRequest,
        slash::{slash_nodes, SlashAccounts, SlashInput},
        verify_statement,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishEquivocationArgs {
    pub statement_a: SignedStatement,
    pub statement_b: SignedStatement,
    /// Must match `equivocation_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
}

pub fn punish_equivocation<'info>(
    ctx: Context<'info, PunishEquivocation<'info>>,
    round_request: RoundRequest,
    args: PunishEquivocationArgs,
) -> Result<()> {
    require!(
        args.statement_a.value != args.statement_b.value,
        MolphaError::EquivocationSameStatement
    );
    require!(
        equivocation_evidence_hash(&round_request, &args.statement_a, &args.statement_b,)
            == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    let resolution = resolve_intersected_signers_accounts(
        ctx.remaining_accounts,
        &ctx.accounts.evidence_registry.as_ref(),
        &args.statement_a.signers_bitmap,
        &args.statement_b.signers_bitmap,
    ).map_err(|_| error!(MolphaError::InvalidNodeAccount))?;
    require!(
        resolution.intersected_bitmap.popcount() > 0,
        MolphaError::GuiltySetEmpty
    );

    verify_statement(&round_request, &args.statement_a, &resolution.signers_a)?;
    verify_statement(&round_request, &args.statement_b, &resolution.signers_b)?;

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
            offence: OffenceKind::Equivocation,
            registry_version: round_request.registry_version,
            evidence_hash: args.evidence_hash,
        },
    )?;

    emit!(EquivocationProven {
        source_id: round_request.source_id,
        registry_version: round_request.registry_version,
        signatures_required: round_request.signatures_required,
        canonical_timestamp: round_request.canonical_timestamp,
        guilty_bitmap: resolution.intersected_bitmap.to_bytes(),
        evidence_hash: args.evidence_hash,
        reporter,
        slashed_total: outcome.slashed_total,
        bounty_paid: outcome.bounty_paid,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(round_request: RoundRequest, args: PunishEquivocationArgs)]
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
        seeds = [Registry::SEED_PREFIX, round_request.registry_version.to_le_bytes().as_ref()],
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
