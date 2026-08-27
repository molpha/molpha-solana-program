//! Node punishment types, fixed penalties, and domain-separated evidence hashes.

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
            OffenceKind::Equivocation => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
            OffenceKind::UnsolicitedSigning => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
            OffenceKind::NonceReuse => Penalty {
                slash_bps: 5_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
        }
    }
}

/// Duty tuple for one statement in a nonce-reuse filing.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigningContext {
    pub source_id: [u8; 32],
    pub signatures_required: u8,
    pub canonical_timestamp: u64,
}

/// Signed statement submitted as punishment evidence.
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

/// Evidence hash for equivocation; also the `PunishmentCase` PDA seed.
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

/// Evidence hash for unsolicited signing; also the `PunishmentCase` PDA seed.
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

/// Hash one bound `(context, statement)` pair.
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

/// Evidence hash for nonce reuse; also the `PunishmentCase` PDA seed.
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
        // Same duty tuple and statements; domain prefix must still separate offence hashes.
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
