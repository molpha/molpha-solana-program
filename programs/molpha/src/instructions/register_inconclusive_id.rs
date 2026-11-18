use anchor_lang::prelude::*;

use crate::{
    error::MolphaError,
    events::InconclusiveIdRegistered,
    state::{inconclusive_id, InconclusiveLink},
};

/// Record that `I(source_id)` is derived from `source_id`, so the committee of an INCONCLUSIVE
/// attestation (signed under `I(S)`) is drawn under `S` by `submit_ticket` and the `UnsolicitedSigning`
/// offence. Permissionless: the address is derived from `I(S)` and the program computes it, so a link
/// can only ever say the truth. A gateway creates it before it dispatches the first round of a source
/// that signs INCONCLUSIVE results, and a node signs `I(S)` only once it exists.
///
/// `link_id` is `I(source_id)`, which the account's address is derived from; the program recomputes it
/// and refuses any other value, so the caller only saves it the derivation in the seeds.
pub fn register_inconclusive_id(
    ctx: Context<RegisterInconclusiveId>,
    source_id: [u8; 32],
    link_id: [u8; 32],
) -> Result<()> {
    require!(source_id != [0u8; 32], MolphaError::InvalidSourceId);
    require!(
        link_id == inconclusive_id(&source_id),
        MolphaError::InvalidInconclusiveLink
    );
    let link = &mut ctx.accounts.inconclusive_link;
    link.source_id = source_id;
    link.bump = ctx.bumps.inconclusive_link;
    emit!(InconclusiveIdRegistered {
        source_id,
        inconclusive_id: link_id,
        payer: ctx.accounts.payer.key(),
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(source_id: [u8; 32], link_id: [u8; 32])]
pub struct RegisterInconclusiveId<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = InconclusiveLink::SPACE,
        seeds = [InconclusiveLink::SEED_PREFIX, link_id.as_ref()],
        bump,
    )]
    pub inconclusive_link: Account<'info, InconclusiveLink>,

    pub system_program: Program<'info, System>,
}
