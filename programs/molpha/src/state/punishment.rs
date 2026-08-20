//! Reusable node-punishment primitives.
//!
//! Node-fault offences share the same sentencing mechanics (a fund penalty plus a registry-
//! membership consequence) — see [`Penalty`]. A new offence only has to prove its own guilty
//! bitmap and pick an [`OffenceKind`]; [`crate::utils::punish::apply_penalty`] and
//! [`crate::utils::punish::evict_nodes`] do the rest.
//!
//! [`OffenceKind::Equivocation`] is the first offence implemented (see `punish_equivocation`):
//! two valid Molpha attestations for the same execution duty
//! (`source_id`, `registry_version`, `signatures_required`, `canonical_timestamp`) committing to
//! different statements. The guilty set is the intersection of the two signer bitmaps — a node
//! in that intersection contributed a partial signature to both, which is only possible if it
//! holds the secret behind its registered secp256k1 key (see the proof-of-possession check in
//! `utils::registry::verify_node_pop`).

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const DOMAIN_EQUIVOCATION: &[u8] = b"MOLPHA_EQUIVOCATION_V1";
pub const DOMAIN_UNSOLICITED_SIGNING: &[u8] = b"MOLPHA_UNSOLICITED_SIGNING_V1";

/// Registry-membership consequence of a punishment.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum MembershipAction {
    /// Pure fine — registry membership untouched.
    None,
    /// Evict from the registry; restorable via `reinstate_node` once thawed.
    Freeze,
    /// Evict from the registry permanently. Terminal — no restoration path.
    Tombstone,
}

/// What a proven offence costs. Shared across offence types.
#[derive(Clone, Copy, Debug)]
pub struct Penalty {
    /// Portion of the node's slashable balances destroyed, in bps. `10_000` = 100%.
    pub slash_bps: u16,
    pub membership: MembershipAction,
    /// For [`MembershipAction::Freeze`]: seconds until permissionless reinstatement is allowed.
    pub freeze_secs: i64,
}

/// Node-fault offence types. Each carries a fixed [`Penalty`] — see [`OffenceKind::penalty`].
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum OffenceKind {
    /// Two valid Molpha attestations for the same execution duty with different statements.
    Equivocation,
    /// A valid Molpha attestation whose signer bitmap contains a node outside the duty's
    /// deterministic selection set.
    UnsolicitedSigning,
}

impl OffenceKind {
    pub const fn penalty(self) -> Penalty {
        match self {
            // 100% slash + permanent tombstone. A plain-sum coalition signature over the
            // intersection bitmap is only producible with every intersection member's secret, so
            // the proof is unambiguous and warrants the maximum sentence.
            OffenceKind::Equivocation => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
            // 100% slash + permanent tombstone. The selection set is a pure function of fields
            // the node itself signed over, so a set bit outside it proves the node contributed a
            // partial signature to a duty it could have checked it was never assigned.
            OffenceKind::UnsolicitedSigning => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
        }
    }
}

/// One signed statement offered as punishment evidence: an aggregate Schnorr signature over
/// `value` for the duty tuple carried alongside it in the offence's args (e.g.
/// `PunishEquivocationArgs`, `PunishUnsolicitedSigningArgs`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedStatement {
    pub value: Vec<u8>,
    /// Aggregate Schnorr scalar `s`.
    pub agg_sig_s: [u8; 32],
    /// Ethereum-style commitment address of the aggregate nonce `R`.
    pub commitment_addr: [u8; 20],
    pub signers_bitmap: [u8; 32],
}

fn statement_hash(statement: &SignedStatement) -> [u8; 32] {
    hashv(&[
        statement.value.as_slice(),
        statement.agg_sig_s.as_slice(),
        statement.commitment_addr.as_slice(),
        statement.signers_bitmap.as_slice(),
    ])
    .to_bytes()
}

/// Evidence commitment for a proven equivocation — also the `PunishmentCase` PDA seed.
///
/// Domain-separated over the shared duty tuple and both statement hashes, with the two statement
/// hashes ordered lexicographically so the same pair of attestations submitted in either order
/// commits to the same case. `PunishmentCase::init` (via the PDA `init` constraint) then rejects
/// a second submission of the same evidence as a replay.
pub fn equivocation_evidence_hash(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: i64,
    statement_a: &SignedStatement,
    statement_b: &SignedStatement,
) -> [u8; 32] {
    let hash_a = statement_hash(statement_a);
    let hash_b = statement_hash(statement_b);
    let (lo, hi) = if hash_a <= hash_b {
        (hash_a, hash_b)
    } else {
        (hash_b, hash_a)
    };
    hashv(&[
        DOMAIN_EQUIVOCATION,
        source_id.as_slice(),
        registry_version.to_be_bytes().as_slice(),
        &[signatures_required],
        (canonical_timestamp as u64).to_be_bytes().as_slice(),
        lo.as_slice(),
        hi.as_slice(),
    ])
    .to_bytes()
}

/// Evidence commitment for a proven unsolicited-signing offence — also the `PunishmentCase` PDA
/// seed. Domain-separated from [`equivocation_evidence_hash`] (distinct prefix) so the two offence
/// types can never collide on the same `PunishmentCase` PDA even given identical duty fields.
pub fn unsolicited_signing_evidence_hash(
    source_id: &[u8; 32],
    registry_version: u32,
    signatures_required: u8,
    canonical_timestamp: i64,
    statement: &SignedStatement,
) -> [u8; 32] {
    hashv(&[
        DOMAIN_UNSOLICITED_SIGNING,
        source_id.as_slice(),
        registry_version.to_be_bytes().as_slice(),
        &[signatures_required],
        (canonical_timestamp as u64).to_be_bytes().as_slice(),
        statement_hash(statement).as_slice(),
    ])
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_statement(value: u8, bitmap_byte: u8) -> SignedStatement {
        SignedStatement {
            value: vec![value; 8],
            agg_sig_s: [0x11; 32],
            commitment_addr: [0x22; 20],
            signers_bitmap: {
                let mut bm = [0u8; 32];
                bm[31] = bitmap_byte;
                bm
            },
        }
    }

    #[test]
    fn equivocation_hash_is_order_independent() {
        let a = fixture_statement(1, 0b0011);
        let b = fixture_statement(2, 0b0101);
        let forward = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);
        let backward = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &b, &a);
        assert_eq!(forward, backward);
    }

    #[test]
    fn equivocation_hash_is_sensitive_to_every_field() {
        let a = fixture_statement(1, 0b0011);
        let b = fixture_statement(2, 0b0101);
        let base = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);

        assert_ne!(
            equivocation_evidence_hash(&[8u8; 32], 4, 3, 1_700_000_000, &a, &b),
            base
        );
        assert_ne!(
            equivocation_evidence_hash(&[7u8; 32], 5, 3, 1_700_000_000, &a, &b),
            base
        );
        assert_ne!(
            equivocation_evidence_hash(&[7u8; 32], 4, 9, 1_700_000_000, &a, &b),
            base
        );
        assert_ne!(
            equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_001, &a, &b),
            base
        );

        let mut a2 = a.clone();
        a2.value[0] ^= 0xff;
        assert_ne!(
            equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a2, &b),
            base
        );
    }

    #[test]
    fn unsolicited_signing_hash_is_sensitive_to_every_field() {
        let a = fixture_statement(1, 0b0011);
        let base = unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a);

        assert_ne!(
            unsolicited_signing_evidence_hash(&[8u8; 32], 4, 3, 1_700_000_000, &a),
            base
        );
        assert_ne!(
            unsolicited_signing_evidence_hash(&[7u8; 32], 5, 3, 1_700_000_000, &a),
            base
        );
        assert_ne!(
            unsolicited_signing_evidence_hash(&[7u8; 32], 4, 9, 1_700_000_000, &a),
            base
        );
        assert_ne!(
            unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_001, &a),
            base
        );

        let mut a2 = a.clone();
        a2.value[0] ^= 0xff;
        assert_ne!(
            unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a2),
            base
        );
    }

    #[test]
    fn equivocation_and_unsolicited_signing_domains_do_not_collide() {
        // Same duty tuple, and the unsolicited-signing statement equals one of the two
        // equivocation statements — the domain prefix alone must still separate them.
        let a = fixture_statement(1, 0b0011);
        let b = fixture_statement(2, 0b0101);
        let equivocation = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);
        let unsolicited = unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a);
        assert_ne!(equivocation, unsolicited);
    }
}
