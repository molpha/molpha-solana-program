//! Permissionless: return a ticket marker's rent once its round can no longer be submitted.

use anchor_lang::prelude::*;

use crate::{error::MolphaError, events::TicketMarkerClosed, state::TicketMarker};

pub fn close_ticket_marker(ctx: Context<CloseTicketMarker>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.ticket_marker.closable_after,
        MolphaError::TicketMarkerNotExpired
    );

    emit!(TicketMarkerClosed {
        ticket_marker: ctx.accounts.ticket_marker.key(),
        submitter: ctx.accounts.submitter.key(),
        closed_at: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct CloseTicketMarker<'info> {
    #[account(mut, close = submitter)]
    pub ticket_marker: Account<'info, TicketMarker>,

    /// CHECK: rent receiver; must be the account that paid for the marker.
    #[account(mut, address = ticket_marker.submitter @ MolphaError::InvalidRentReceiver)]
    pub submitter: UncheckedAccount<'info>,
}
