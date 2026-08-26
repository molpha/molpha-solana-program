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
//! `molpha_verifier::validate_key_and_verify_pop`).
//!
//! [`OffenceKind::NonceReuse`] (see `punish_nonce_reuse`) is the third offence: a node
//! contributes a partial signature under the *same* aggregate nonce commitment
//! (`SignedStatement::commitment_addr`) for two different signed messages. The verifier's
//! challenge is `e = keccak(P.x || parity || msg_hash || commitment_addr) mod n`
//! (`molpha_verifier::scalar`), and `commitment_addr` is a function of the nonce alone — so a
//! node cannot compute its partial share without first knowing which commitment it is signing
//! under, and refusing a second, different message under a commitment it has already used is a
//! purely local, always-checkable rule. Because `compute_message_hash` commits to
//! `signers_bitmap`, two statements sharing a `commitment_addr` with different message hashes
//! necessarily have `e_a != e_b`, which is exactly the offence's predicate (and, since the pair
//! `(commitment_addr, msg_hash)` pins `s` uniquely, it also rules out the pair being the same
//! statement submitted twice). The guilty set is the intersection of the two signer bitmaps, as
//! with equivocation.
//!
//! Unlike the first two offences, the chain cannot tell *why* the commitment collided: either the
//! node reused its own nonce share (its partials then satisfy `s_i1 - s_i2 = (e1 - e2) * x_i`,
//! so its secret key is already recoverable by anyone who assembled both aggregates), or two
//! cosigners offset their shares so only the *aggregate* nonce collided while neither reused its
//! own (keys possibly intact). The sentence assumes the worse case for membership — tombstone,
//! since the key may already be compromised — but only the provable minimum for funds — a 50%
//! slash for a locally-preventable protocol violation, not the 100% reserved for the other two
//! offences' unambiguous proof of malice. (Full coalition-secret extraction, and the "recovers
//! every other member's key" framing, only holds when the two statements share an identical
//! signer bitmap; with differing bitmaps the guilty set is still sound — every intersection
//! member demonstrably signed twice under one commitment — but the coalition secret is not
//! algebraically recoverable from the aggregates alone.)

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

pub const DOMAIN_EQUIVOCATION: &[u8] = b"MOLPHA_EQUIVOCATION_V1";
pub const DOMAIN_UNSOLICITED_SIGNING: &[u8] = b"MOLPHA_UNSOLICITED_SIGNING_V1";
pub const DOMAIN_NONCE_REUSE: &[u8] = b"MOLPHA_NONCE_REUSE_V1";

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
    /// A node contributed a partial signature under one aggregate nonce commitment for two
    /// different signed messages.
    NonceReuse,
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
            // 50% slash + permanent tombstone — the first partial-slash offence. Signing twice
            // under one aggregate nonce commitment is locally preventable (see the module doc),
            // but the chain cannot distinguish a node that reused its own nonce share (secret key
            // already recoverable) from two cosigners that merely offset each other's shares (the
            // aggregate collided, but neither node's own key is implicated). Tombstone assumes the
            // worse case, since the key may already be compromised and must never be used again;
            // the slash is only half because the provable minimum is a locally-preventable
            // protocol violation, not the unambiguous malice the other two offences establish.
            OffenceKind::NonceReuse => Penalty {
                slash_bps: 5_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
        }
    }
}

/// The duty tuple one statement in a [`nonce_reuse_evidence_hash`] filing was signed under.
///
/// Distinct from equivocation and unsolicited-signing, where both statements in a filing always
/// share one duty tuple: nonce reuse spans two signing contexts by definition, so each statement
/// carries its own. `registry_version` is deliberately absent — both statements in a nonce-reuse
/// filing must be signed against the same registry version (cross-version reuse is out of scope,
/// mirroring the equivocation module doc's own cross-version limitation), so it lives once on
/// `PunishNonceReuseArgs` instead, where the pinned `evidence_registry` seed constraint needs it.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigningContext {
    pub source_id: [u8; 32],
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
}

/// One signed statement offered as punishment evidence: an aggregate Schnorr signature over
/// `value` for the duty tuple carried alongside it in the offence's args (e.g.
/// `PunishEquivocationArgs`, `PunishUnsolicitedSigningArgs`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedStatement {
    pub value: [u8; 32],
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
    canonical_timestamp: u64,
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
        signatures_required.to_be_bytes().as_slice(),
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
    canonical_timestamp: u64,
    statement: &SignedStatement,
) -> [u8; 32] {
    hashv(&[
        DOMAIN_UNSOLICITED_SIGNING,
        source_id.as_slice(),
        registry_version.to_be_bytes().as_slice(),
        signatures_required.to_be_bytes().as_slice(),
        (canonical_timestamp as u64).to_be_bytes().as_slice(),
        statement_hash(statement).as_slice(),
    ])
    .to_bytes()
}

/// Hash one `(context, statement)` pair for [`nonce_reuse_evidence_hash`].
///
/// Pairing keeps each statement bound to its own duty tuple in the evidence commitment — hashing
/// the two contexts and the two statements independently would let a filer pair statement A with
/// context B and still land on a valid-looking case.
fn context_statement_hash(context: &SigningContext, statement: &SignedStatement) -> [u8; 32] {
    hashv(&[
        context.source_id.as_slice(),
        context.signatures_required.to_be_bytes().as_slice(),
        (context.canonical_timestamp as u64)
            .to_be_bytes()
            .as_slice(),
        statement_hash(statement).as_slice(),
    ])
    .to_bytes()
}

/// Evidence commitment for a proven nonce-reuse offence — also the `PunishmentCase` PDA seed.
///
/// Unlike [`equivocation_evidence_hash`], the two statements do not share one duty tuple, so
/// ordering happens over `(context, statement)` pairs (via [`context_statement_hash`]) rather than
/// over bare statement hashes: submitting the same two pairs in either order commits to the same
/// case, while swapping only the statements between the two contexts does not. Domain-separated
/// from [`equivocation_evidence_hash`] and [`unsolicited_signing_evidence_hash`] by both prefix
/// and preimage length, so no choice of inputs can collide two of the three.
pub fn nonce_reuse_evidence_hash(
    registry_version: u32,
    context_a: &SigningContext,
    statement_a: &SignedStatement,
    context_b: &SigningContext,
    statement_b: &SignedStatement,
) -> [u8; 32] {
    let hash_a = context_statement_hash(context_a, statement_a);
    let hash_b = context_statement_hash(context_b, statement_b);
    let (lo, hi) = if hash_a <= hash_b {
        (hash_a, hash_b)
    } else {
        (hash_b, hash_a)
    };
    hashv(&[
        DOMAIN_NONCE_REUSE,
        registry_version.to_be_bytes().as_slice(),
        lo.as_slice(),
        hi.as_slice(),
    ])
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_statement(value: u8, bitmap_byte: u8) -> SignedStatement {
        SignedStatement {
            value: [value; 32],
            agg_sig_s: [0x11; 32],
            commitment_addr: [0x22; 20],
            signers_bitmap: {
                let mut bm = [0u8; 32];
                bm[31] = bitmap_byte;
                bm
            },
        }
    }

    fn fixture_context(source_byte: u8, signatures_required: u8, ts: u64) -> SigningContext {
        SigningContext {
            source_id: [source_byte; 32],
            signatures_required,
            canonical_timestamp: ts,
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
    fn offence_evidence_domains_do_not_collide() {
        // Same duty tuple everywhere, and the unsolicited-signing / nonce-reuse statements reuse
        // the equivocation statements exactly — the domain prefix (and, redundantly, the distinct
        // preimage length of each hash) must still separate all three.
        let a = fixture_statement(1, 0b0011);
        let b = fixture_statement(2, 0b0101);
        let ctx_a = fixture_context(7, 3, 1_700_000_000);
        let ctx_b = fixture_context(7, 3, 1_700_000_000);

        let equivocation = equivocation_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a, &b);
        let unsolicited = unsolicited_signing_evidence_hash(&[7u8; 32], 4, 3, 1_700_000_000, &a);
        let nonce_reuse = nonce_reuse_evidence_hash(4, &ctx_a, &a, &ctx_b, &b);

        assert_ne!(equivocation, unsolicited);
        assert_ne!(equivocation, nonce_reuse);
        assert_ne!(unsolicited, nonce_reuse);
    }

    #[test]
    fn nonce_reuse_hash_is_pairwise_order_independent() {
        let ctx_a = fixture_context(7, 3, 1_700_000_000);
        let stmt_a = fixture_statement(1, 0b0011);
        let ctx_b = fixture_context(9, 4, 1_700_000_100);
        let stmt_b = fixture_statement(2, 0b0101);

        let forward = nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);
        let backward = nonce_reuse_evidence_hash(4, &ctx_b, &stmt_b, &ctx_a, &stmt_a);
        assert_eq!(forward, backward);
    }

    #[test]
    fn nonce_reuse_hash_pairs_each_context_with_its_own_statement() {
        // Swapping only the statements between the two contexts (as opposed to swapping the whole
        // pairs) must NOT commit to the same hash — otherwise a filer could pair statement A with
        // context B.
        let ctx_a = fixture_context(7, 3, 1_700_000_000);
        let stmt_a = fixture_statement(1, 0b0011);
        let ctx_b = fixture_context(9, 4, 1_700_000_100);
        let stmt_b = fixture_statement(2, 0b0101);

        let paired = nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);
        let cross_paired = nonce_reuse_evidence_hash(4, &ctx_a, &stmt_b, &ctx_b, &stmt_a);
        assert_ne!(paired, cross_paired);
    }

    #[test]
    fn nonce_reuse_hash_is_sensitive_to_every_field() {
        let ctx_a = fixture_context(7, 3, 1_700_000_000);
        let stmt_a = fixture_statement(1, 0b0011);
        let ctx_b = fixture_context(9, 4, 1_700_000_100);
        let stmt_b = fixture_statement(2, 0b0101);
        let base = nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b, &stmt_b);

        assert_ne!(
            nonce_reuse_evidence_hash(5, &ctx_a, &stmt_a, &ctx_b, &stmt_b),
            base
        );

        let mut ctx_a2 = ctx_a;
        ctx_a2.source_id = [8u8; 32];
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a2, &stmt_a, &ctx_b, &stmt_b),
            base
        );
        let mut ctx_a3 = ctx_a;
        ctx_a3.signatures_required = 9;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a3, &stmt_a, &ctx_b, &stmt_b),
            base
        );
        let mut ctx_a4 = ctx_a;
        ctx_a4.canonical_timestamp += 1;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a4, &stmt_a, &ctx_b, &stmt_b),
            base
        );

        let mut ctx_b2 = ctx_b;
        ctx_b2.source_id = [10u8; 32];
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b2, &stmt_b),
            base
        );
        let mut ctx_b3 = ctx_b;
        ctx_b3.signatures_required = 6;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b3, &stmt_b),
            base
        );
        let mut ctx_b4 = ctx_b;
        ctx_b4.canonical_timestamp += 1;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a, &ctx_b4, &stmt_b),
            base
        );

        let mut stmt_a2 = stmt_a.clone();
        stmt_a2.value[0] ^= 0xff;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a2, &ctx_b, &stmt_b),
            base
        );
        let mut stmt_a3 = stmt_a.clone();
        stmt_a3.agg_sig_s[0] ^= 0xff;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a3, &ctx_b, &stmt_b),
            base
        );
        let mut stmt_a4 = stmt_a.clone();
        stmt_a4.commitment_addr[0] ^= 0xff;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a4, &ctx_b, &stmt_b),
            base
        );
        let mut stmt_a5 = stmt_a.clone();
        stmt_a5.signers_bitmap[0] ^= 0xff;
        assert_ne!(
            nonce_reuse_evidence_hash(4, &ctx_a, &stmt_a5, &ctx_b, &stmt_b),
            base
        );
    }

    #[test]
    fn offence_kind_discriminants_are_stable() {
        // `PunishmentCase::offence` is borsh-encoded on-chain; the discriminant of an existing
        // variant must never change, and a new variant must only ever be appended.
        assert_eq!(borsh::to_vec(&OffenceKind::Equivocation).unwrap(), vec![0]);
        assert_eq!(
            borsh::to_vec(&OffenceKind::UnsolicitedSigning).unwrap(),
            vec![1]
        );
        assert_eq!(borsh::to_vec(&OffenceKind::NonceReuse).unwrap(), vec![2]);
    }

    #[test]
    fn penalties_are_pinned() {
        let equivocation = OffenceKind::Equivocation.penalty();
        assert_eq!(equivocation.slash_bps, 10_000);
        assert_eq!(equivocation.membership, MembershipAction::Tombstone);
        assert_eq!(equivocation.freeze_secs, 0);

        let unsolicited = OffenceKind::UnsolicitedSigning.penalty();
        assert_eq!(unsolicited.slash_bps, 10_000);
        assert_eq!(unsolicited.membership, MembershipAction::Tombstone);
        assert_eq!(unsolicited.freeze_secs, 0);

        let nonce_reuse = OffenceKind::NonceReuse.penalty();
        assert_eq!(nonce_reuse.slash_bps, 5_000);
        assert_eq!(nonce_reuse.membership, MembershipAction::Tombstone);
        assert_eq!(nonce_reuse.freeze_secs, 0);
    }
}
