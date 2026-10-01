//! Permissionless: submit a sampled round as participation evidence for its epoch.

use anchor_lang::prelude::*;
use molpha_verifier::{selection::derive_selection_bitmap, Attestation, Bitmap, CoalitionKey};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::{TicketRosterOverflow, TicketSubmitted},
    state::{
        dispatch_message, ticket_hash, ticket_id, EpochState, Gateway, GatewayStatus,
        ProtocolConfig, Registry, TicketMarker, MAX_TICKETS_PER_EPOCH,
    },
    utils::{
        attestation::{require_registry_active_at, verify_attestation_core},
        verify_ed25519_signature, MAX_FUTURE_DRIFT_SECONDS,
    },
};

/// A round is a ticket when `ticket_hash` falls under the epoch's pinned threshold. The ticket
/// must carry the gateway's dispatch signature (Ed25519 instruction immediately before this one),
/// which binds it to a round a registered gateway admitted. Its full selection is re-derived and
/// added to the epoch counters; signer `Node` accounts are read-only.
///
/// - `epoch_index`: the epoch whose window contains `attestation.payload.canonical_timestamp`.
/// - `roster_slots`: epoch roster slot of each selected registry position, in ascending bit
///   order: an existing slot, or the current `roster_len` for a node the epoch has not seen yet.
/// - `coalition_key`: affine sum of the signers' keys; untrusted, checked by the verifier.
pub fn submit_ticket<'info>(
    ctx: Context<'info, SubmitTicket<'info>>,
    epoch_index: u64,
    attestation: Attestation,
    roster_slots: Vec<u16>,
    coalition_key: CoalitionKey,
) -> Result<()> {
    let payload = &attestation.payload;
    let canonical_timestamp = payload.canonical_timestamp;
    let now = Clock::get()?.unix_timestamp;

    // Cheap window and lottery checks before any signature work.
    {
        let epoch = ctx.accounts.epoch_state.load()?;
        require!(epoch.accepts_tickets(now), MolphaError::TicketWindowClosed);
        require!(
            epoch.contains(canonical_timestamp),
            MolphaError::RoundOutsideEpoch
        );
        require!(
            epoch.tickets < MAX_TICKETS_PER_EPOCH,
            MolphaError::TicketCapReached
        );
        require!(
            epoch.is_ticket(&ticket_hash(&attestation)),
            MolphaError::NotATicket
        );
    }
    require!(
        canonical_timestamp
            <= u64::try_from(now)
                .unwrap_or(0)
                .saturating_add(MAX_FUTURE_DRIFT_SECONDS),
        MolphaError::AttestationInFuture
    );

    // The version must have been live when the round ran, not when the ticket lands: tickets
    // arrive up to an epoch plus grace later.
    require_registry_active_at(&*ctx.accounts.registry.load()?, canonical_timestamp as i64)?;

    require!(
        verify_ed25519_signature(
            &ctx.accounts.instructions_sysvar,
            &ctx.accounts.gateway.authority.to_bytes(),
            &dispatch_message(&crate::ID, payload),
        )?,
        MolphaError::DispatchSigInvalid
    );

    let min_signers = ctx.accounts.epoch_state.load()?.min_signers;
    verify_attestation_core(
        ctx.remaining_accounts,
        &ctx.accounts.registry,
        min_signers,
        &attestation,
        &coalition_key,
    )?;

    let registry = ctx.accounts.registry.load()?;
    let selection = derive_selection_bitmap(
        &payload.source_id,
        canonical_timestamp,
        payload.signatures_required,
        &registry.view(),
    )
    .map_err(map_attestation_err)?;
    let signers = Bitmap::load(&attestation.signature.signers_bitmap);

    let mut epoch = ctx.accounts.epoch_state.load_mut()?;
    let uncounted = epoch.record_ticket(&registry, selection, &signers, &roster_slots)?;
    epoch.tickets += 1;

    let id = ticket_id(payload);
    let marker = &mut ctx.accounts.ticket_marker;
    marker.closable_after = epoch.ticket_deadline();
    marker.submitter = ctx.accounts.submitter.key();
    marker.bump = ctx.bumps.ticket_marker;

    emit!(TicketSubmitted {
        epoch: epoch_index,
        ticket_id: id,
        gateway: ctx.accounts.gateway.key(),
        submitter: marker.submitter,
        registry_version: payload.registry_version,
        signers_bitmap: attestation.signature.signers_bitmap,
    });
    if uncounted > 0 {
        emit!(TicketRosterOverflow {
            epoch: epoch_index,
            ticket_id: id,
            uncounted,
        });
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction(epoch_index: u64, attestation: Attestation)]
pub struct SubmitTicket<'info> {
    #[account(mut)]
    pub submitter: Signer<'info>,

    #[account(seeds = [ProtocolConfig::SEED_PREFIX], bump = protocol_config.bump)]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [Registry::SEED_PREFIX, attestation.payload.registry_version.to_le_bytes().as_ref()],
        bump = registry.load()?.bump,
    )]
    pub registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [EpochState::SEED_PREFIX, epoch_index.to_le_bytes().as_ref()],
        bump = epoch_state.load()?.bump,
    )]
    pub epoch_state: AccountLoader<'info, EpochState>,

    /// Gateway whose authority signed the round dispatch.
    #[account(
        seeds = [Gateway::SEED_PREFIX, gateway.authority.as_ref()],
        bump = gateway.bump,
        constraint = gateway.status == GatewayStatus::Active @ MolphaError::GatewayNotActive,
    )]
    pub gateway: Account<'info, Gateway>,

    #[account(
        init,
        payer = submitter,
        space = TicketMarker::SPACE,
        seeds = [TicketMarker::SEED_PREFIX, &ticket_id(&attestation.payload)],
        bump,
    )]
    pub ticket_marker: Account<'info, TicketMarker>,

    /// CHECK: validated by address; read via Ed25519 instruction introspection.
    #[account(
        address = solana_instructions_sysvar::id() @ MolphaError::InvalidInstructionsSysvar
    )]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
