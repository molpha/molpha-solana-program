//! Shared node punishment: apply penalties and verify evidence signatures.
//!
//! A filing settles in one instruction. Because every node's principal sits in one shared vault,
//! the batch sweeps its whole forfeiture with a single transfer and funds the reporter's bounty
//! from it immediately, so the transaction costs one account per signer and nothing per guilty
//! node.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use molpha_verifier::bitmap::{for_each_set_bit, Bitmap};
use molpha_verifier::Attestation;
use molpha_verifier::{
    derive_selection_bitmap, verify_aggregate_over_hash_with_coalition_key, CoalitionKey,
    RegistryView, SignerXy,
};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::NodePunished,
    state::{Node, NodeStatus, Offence},
};

/// Whether two attestations sign different values for one duty — every payload field but `value`
/// matches.
pub fn is_equivocation(attestation_a: &Attestation, attestation_b: &Attestation) -> bool {
    let (a, b) = (&attestation_a.payload, &attestation_b.payload);
    let same_duty = a.source_id == b.source_id
        && a.registry_version == b.registry_version
        && a.signatures_required == b.signatures_required
        && a.canonical_timestamp == b.canonical_timestamp;

    same_duty && a.value != b.value
}

pub fn attestation_hash(attestation: &Attestation) -> [u8; 32] {
    let (payload, signature) = (&attestation.payload, &attestation.signature);
    hashv(&[
        payload.value.as_slice(),
        payload.source_id.as_slice(),
        payload.registry_version.to_be_bytes().as_slice(),
        payload.signatures_required.to_be_bytes().as_slice(),
        payload.canonical_timestamp.to_be_bytes().as_slice(),
        signature.agg_sig_s.as_slice(),
        signature.commitment.as_slice(),
        signature.signers_bitmap.as_slice(),
    ])
    .to_bytes()
}

/// Lexicographically order two 32-byte digests so pair-wise evidence is attestation-order
/// independent.
pub fn canonical_pair(hash_a: [u8; 32], hash_b: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    if hash_a <= hash_b {
        (hash_a, hash_b)
    } else {
        (hash_b, hash_a)
    }
}

/// Outcome of a [`slash_guilty_nodes`] batch.
#[derive(Clone, Debug, Default)]
pub struct PunishmentBatch {
    /// Nodes slashed in resolution order; may be shorter than `guilty_bitmap` if some are tombstoned.
    pub victims: Vec<Pubkey>,
    /// Principal deducted from the victims and swept into the treasury.
    pub slashed_locked: u64,
    /// Reward credits cancelled, and so released from `ProtocolConfig.reward_liability`.
    pub slashed_rewards: u64,
}

impl PunishmentBatch {
    pub fn slashed_total(&self) -> u64 {
        self.slashed_locked.saturating_add(self.slashed_rewards)
    }
}

/// Slash every guilty node, accumulating what the batch forfeits.
///
/// Already-tombstoned nodes are skipped so one stale bit cannot block the batch. Nothing else is
/// skipped: the whole sentence settles in this instruction, so there is no half-settled state a
/// later offence could collide with, and no way for a node to become temporarily unpunishable.
pub(crate) fn slash_guilty_nodes<'info>(
    input: &SlashInput<'info, '_>,
    withdrawal_cooldown_slots: u64,
) -> Result<PunishmentBatch> {
    let mut batch = PunishmentBatch::default();
    let clock = Clock::get()?;
    let kind = input.offence.kind();

    let mut cursor = 0usize;
    for_each_set_bit(input.signers_bitmap, |bit_pos| {
        let account_info = &input.signer_accounts[cursor];
        cursor += 1;

        if !input.guilty_bitmap.bit_set(bit_pos) {
            return Ok(());
        }

        let mut node: Account<Node> = Account::try_from(account_info)?;

        if node.status == NodeStatus::Tombstoned {
            return Ok(());
        }

        let outcome = input
            .offence
            .penalty()
            .apply(&mut node, clock.unix_timestamp)?;
        if node.status == NodeStatus::Tombstoned {
            node.withdrawable_slot = clock
                .slot
                .checked_add(withdrawal_cooldown_slots)
                .ok_or(MolphaError::WithdrawalArithmeticOverflow)?;
        }

        batch.slashed_locked = batch.slashed_locked.saturating_add(outcome.slashed_locked);
        batch.slashed_rewards = batch
            .slashed_rewards
            .saturating_add(outcome.slashed_rewards);

        emit!(NodePunished {
            node: node.key(),
            authority: node.authority,
            offence: kind,
            slashed_locked: outcome.slashed_locked,
            slashed_rewards: outcome.slashed_rewards,
            new_status: node.status,
            punished_at: clock.unix_timestamp,
        });
        batch.victims.push(node.key());
        node.exit(&Node::owner())?;

        Ok::<(), Error>(())
    })?;

    Ok(batch)
}

/// Proof output from an offence handler, fed into [`slash_guilty_nodes`].
pub struct SlashInput<'info, 'a> {
    /// The signer `Node` accounts the aggregate was verified against, in ascending bit order.
    pub signer_accounts: &'info [AccountInfo<'info>],
    pub signers_bitmap: Bitmap,
    pub guilty_bitmap: Bitmap,
    pub offence: &'a Offence,
}

/// Require an evidence attestation's signers to sit inside the selection derived for its duty.
///
/// Commitment reuse uses this so a selected coalition cannot be punished for an unsolicited
/// signer's `R`. Equivocation does not: its gate is the two verifying aggregates, and unsolicited
/// bits in that pair are filed separately. Quorum is not required.
pub(crate) fn require_signers_selected(
    attestation: &Attestation,
    registry: &RegistryView,
) -> Result<()> {
    let selection = derive_selection_bitmap(
        &attestation.payload.source_id,
        attestation.payload.canonical_timestamp,
        attestation.payload.signatures_required,
        registry,
    )
    .map_err(map_attestation_err)?;
    require!(
        Bitmap::load(&attestation.signature.signers_bitmap).is_subset(&selection),
        MolphaError::SignersNotSubsetOfSelection
    );
    Ok(())
}

/// Verify an evidence attestation's aggregate signature against its resolved signers.
///
/// Signature only — unlike `molpha_verifier::verify`, neither selection membership nor quorum is
/// enforced, since unsolicited-signing evidence breaks the former by definition. Callers that
/// need membership check it with [`require_signers_selected`].
///
/// `coalition_key` is the reporter-supplied affine sum of `ordered_signers`; a wrong one fails
/// the filing with `InvalidCoalitionKey` rather than reading as an invalid signature.
pub(crate) fn verify_evidence_signature(
    attestation: &Attestation,
    ordered_signers: &[SignerXy],
    coalition_key: &CoalitionKey,
) -> Result<()> {
    let is_valid = verify_aggregate_over_hash_with_coalition_key(
        &attestation.signature.agg_sig_s,
        &attestation.signature.commitment,
        &attestation.message_hash(),
        ordered_signers,
        coalition_key,
    )
    .map_err(map_attestation_err)?;
    require!(is_valid, MolphaError::InvalidAggregateSignature);
    Ok(())
}
