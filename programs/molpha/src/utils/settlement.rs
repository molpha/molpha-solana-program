use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use molpha_verifier::{
    bitmap::{for_each_set_bit, Bitmap},
    selection::derive_selection_bitmap,
    Attestation,
};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::NodeLivelinessPenalized,
    state::{Node, ProtocolConfig, Registry},
};

pub(crate) fn credit_round_rewards<'info>(
    registry_acc: &AccountLoader<'info, Registry>,
    config: &mut Account<'info, ProtocolConfig>,
    treasury: &Account<'info, TokenAccount>,
    remaining_accounts: &'info [AccountInfo<'info>],
    attestation: &Attestation,
    round_replay: Pubkey,
) -> Result<()> {
    let registry = registry_acc.load()?;

    // Derive the complete selection so this instruction can update both signers and misses.
    let selection = derive_selection_bitmap(
        &attestation.payload.source_id,
        attestation.payload.canonical_timestamp,
        attestation.payload.signatures_required,
        &registry.view(),
    )
    .map_err(map_attestation_err)?;

    require!(
        remaining_accounts.len() == selection.popcount() as usize,
        MolphaError::InvalidMissedNodeAccount
    );

    let signers = Bitmap::load(&attestation.signature.signers_bitmap);
    require!(
        signers.is_subset(&selection),
        MolphaError::SignersNotSubsetOfSelection
    );

    let signer_count = signers.popcount();
    config.record_reward_liability(signer_count)?;
    require!(
        treasury.amount >= config.reward_liability,
        MolphaError::TreasuryInsolvent
    );

    let mut cursor = 0usize;
    for_each_set_bit(signers, |bit_pos| {
        let node_ai = &remaining_accounts[cursor];
        cursor += 1;

        let mut node: Account<Node> = Account::try_from(node_ai)?;
        require_keys_eq!(
            node_ai.key(),
            registry.node(bit_pos)?,
            MolphaError::InvalidCreditNodeAccount
        );

        node.record_round_settlement(config)?;
        node.exit(&Node::owner())?;

        Ok(())
    })?;

    let missed = selection.subtract(&signers);

    for_each_set_bit(missed, |bit_pos| {
        let node_ai = &remaining_accounts[cursor];
        cursor += 1;

        require_keys_eq!(
            node_ai.key(),
            registry.node(bit_pos)?,
            MolphaError::InvalidMissedNodeAccount
        );
        let mut node: Account<Node> = Account::try_from(node_ai)?;

        node.liveliness_score = node
            .liveliness_score
            .saturating_sub(config.liveliness_penalty_per_miss);
        emit!(NodeLivelinessPenalized {
            node: node.key(),
            round_replay,
            score: node.liveliness_score,
        });
        node.exit(&Node::owner())?;

        Ok(())
    })?;

    Ok(())
}
