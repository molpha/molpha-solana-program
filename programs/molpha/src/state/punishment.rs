//! Node punishment types, fixed penalties, and domain-separated evidence hashes.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::state::CommittedNoncePair;
use crate::utils::settlement::RoundRequest;

pub const DOMAIN_EQUIVOCATION: &[u8] = b"MOLPHA_EQUIVOCATION_V1";
pub const DOMAIN_UNSOLICITED_SIGNING: &[u8] = b"MOLPHA_UNSOLICITED_SIGNING_V1";
pub const DOMAIN_COMMITMENT_REUSE: &[u8] = b"MOLPHA_COMMITMENT_REUSE_V1";
pub const DOMAIN_NONCE_REUSE: &[u8] = b"MOLPHA_NONCE_REUSE_V1";

/// Registry-membership consequence of a punishment.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum MembershipAction {
    None,
    Freeze,
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
    CommitmentReuse,
    /// A node produced two verifying partial signatures against one nonce pair it had committed
    /// in a signed batch. Key-compromise-equivalent: a third such partial extracts the share.
    NonceReuse,
    /// A node liveliness score dropped below the freeze threshold.
    Inactivity,
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
            OffenceKind::CommitmentReuse => Penalty {
                slash_bps: 5_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
            OffenceKind::NonceReuse => Penalty {
                slash_bps: 10_000,
                membership: MembershipAction::Tombstone,
                freeze_secs: 0,
            },
            OffenceKind::Inactivity => Penalty {
                slash_bps: 0,
                membership: MembershipAction::Freeze,
                freeze_secs: 86_400,
            },
        }
    }
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

/// Lexicographically order two 32-byte digests so pair-wise evidence is statement-order independent.
fn canonical_pair(hash_a: [u8; 32], hash_b: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    if hash_a <= hash_b {
        (hash_a, hash_b)
    } else {
        (hash_b, hash_a)
    }
}

/// Evidence hash for equivocation; also the `PunishmentCase` PDA seed.
///
/// Statement hashes are order-normalized so swapping the two attestations cannot open a second
/// case over one offence.
pub fn equivocation_evidence_hash(
    round_request: &RoundRequest,
    statement_a: &SignedStatement,
    statement_b: &SignedStatement,
) -> [u8; 32] {
    let (lo, hi) = canonical_pair(
        statement_hash(statement_a),
        statement_hash(statement_b),
    );
    hashv(&[
        DOMAIN_EQUIVOCATION,
        round_request.source_id.as_slice(),
        round_request.registry_version.to_be_bytes().as_slice(),
        round_request.signatures_required.to_be_bytes().as_slice(),
        round_request.canonical_timestamp.to_be_bytes().as_slice(),
        lo.as_slice(),
        hi.as_slice(),
    ])
    .to_bytes()
}

/// Evidence hash for unsolicited signing; also the `PunishmentCase` PDA seed.
pub fn unsolicited_signing_evidence_hash(
    round_request: &RoundRequest,
    statement: &SignedStatement,
) -> [u8; 32] {
    hashv(&[
        DOMAIN_UNSOLICITED_SIGNING,
        round_request.source_id.as_slice(),
        round_request.registry_version.to_be_bytes().as_slice(),
        round_request.signatures_required.to_be_bytes().as_slice(),
        round_request.canonical_timestamp.to_be_bytes().as_slice(),
        statement_hash(statement).as_slice(),
    ])
    .to_bytes()
}

/// Hash one bound `(context, statement)` pair.
fn context_statement_hash(round_request: &RoundRequest, statement: &SignedStatement) -> [u8; 32] {
    hashv(&[
        round_request.source_id.as_slice(),
        round_request.registry_version.to_be_bytes().as_slice(),
        round_request.signatures_required.to_be_bytes().as_slice(),
        round_request.canonical_timestamp.to_be_bytes().as_slice(),
        statement_hash(statement).as_slice(),
    ])
    .to_bytes()
}

/// Evidence hash for commitment reuse; also the `PunishmentCase` PDA seed.
pub fn commitment_reuse_evidence_hash(
    round_request_a: &RoundRequest,
    statement_a: &SignedStatement,
    round_request_b: &RoundRequest,
    statement_b: &SignedStatement,
) -> [u8; 32] {
    let (lo, hi) = canonical_pair(
        context_statement_hash(round_request_a, statement_a),
        context_statement_hash(round_request_b, statement_b),
    );
    hashv(&[
        DOMAIN_COMMITMENT_REUSE,
        lo.as_slice(),
        hi.as_slice(),
    ])
    .to_bytes()
}

/// One partial-signature transcript: the challenge scalars and the resulting share.
///
/// The program never recomputes `b` or `c` from a signing context — the offence is defined purely
/// by the equation `s*G == R1 + b*R2 + c*X`, which is why message transport and commitment lists
/// stay off-chain.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartialTranscript {
    /// Binding factor.
    pub b: [u8; 32],
    /// Challenge.
    pub c: [u8; 32],
    /// The node's partial signature scalar.
    pub s: [u8; 32],
}

fn partial_transcript_hash(transcript: &PartialTranscript) -> [u8; 32] {
    hashv(&[
        transcript.b.as_slice(),
        transcript.c.as_slice(),
        transcript.s.as_slice(),
    ])
    .to_bytes()
}

/// Evidence hash for nonce-slot reuse; also the `PunishmentCase` PDA seed.
///
/// Covers the offence identity only: which node, under which registry version, reused which
/// committed slot, with which two transcripts. The merkle path and batch signature are
/// verification witnesses rather than identity, so they are excluded — a given leaf under a signed
/// root has exactly one path anyway. Transcript hashes are order-normalized so that swapping the
/// two transcripts cannot open a second case over one offence.
#[allow(clippy::too_many_arguments)]
pub fn nonce_reuse_evidence_hash(
    registry_version: u32,
    node_index: u32,
    epoch: u64,
    batch_meta: &[u8; 32],
    batch_root: &[u8; 32],
    slot: u64,
    nonces: &CommittedNoncePair,
    transcript_a: &PartialTranscript,
    transcript_b: &PartialTranscript,
) -> [u8; 32] {
    let (lo, hi) = canonical_pair(
        partial_transcript_hash(transcript_a),
        partial_transcript_hash(transcript_b),
    );
    hashv(&[
        DOMAIN_NONCE_REUSE,
        registry_version.to_be_bytes().as_slice(),
        node_index.to_be_bytes().as_slice(),
        epoch.to_be_bytes().as_slice(),
        batch_meta.as_slice(),
        batch_root.as_slice(),
        slot.to_be_bytes().as_slice(),
        nonces.r1_x.as_slice(),
        nonces.r1_y.as_slice(),
        nonces.r2_x.as_slice(),
        nonces.r2_y.as_slice(),
        lo.as_slice(),
        hi.as_slice(),
    ])
    .to_bytes()
}
