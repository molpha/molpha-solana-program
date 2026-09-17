//! Node punishment types, fixed penalties, and domain-separated evidence hashes.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::state::CommittedNoncePair;
use molpha_verifier::Attestation;

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
    /// Two valid Molpha attestations for the same execution duty with different values.
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

pub trait Evidence {
    fn evidence_hash(&self) -> [u8; 32];
    fn offence_kind(&self) -> OffenceKind;
    fn penalty(&self) -> Penalty;
    fn registry_version(&self) -> u32;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct EquivocationEvidence {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
}

impl Evidence for EquivocationEvidence {
    #[inline]
    fn evidence_hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(
            attestation_hash(&self.attestation_a),
            attestation_hash(&self.attestation_b),
        );
        hashv(&[DOMAIN_EQUIVOCATION, lo.as_slice(), hi.as_slice()]).to_bytes()
    }

    fn offence_kind(&self) -> OffenceKind {
        OffenceKind::Equivocation
    }

    fn penalty(&self) -> Penalty {
        self.offence_kind().penalty()
    }

    fn registry_version(&self) -> u32 {
        // Pair offences validate version equality in the handler; account resolution uses the
        // first attestation's version so a mismatched filing fails with `RegistryVersionMismatch`
        // instead of aborting during constraint evaluation.
        self.attestation_a.payload.registry_version
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CommitmentReuseEvidence {
    pub attestation_a: Attestation,
    pub attestation_b: Attestation,
}

impl Evidence for CommitmentReuseEvidence {
    #[inline]
    fn evidence_hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(
            attestation_hash(&self.attestation_a),
            attestation_hash(&self.attestation_b),
        );
        hashv(&[DOMAIN_COMMITMENT_REUSE, lo.as_slice(), hi.as_slice()]).to_bytes()
    }

    fn offence_kind(&self) -> OffenceKind {
        OffenceKind::CommitmentReuse
    }

    fn penalty(&self) -> Penalty {
        self.offence_kind().penalty()
    }

    fn registry_version(&self) -> u32 {
        self.attestation_a.payload.registry_version
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct NonceReuseEvidence {
    /// Registry version that `node_index` indexes into; also bound into the batch signature.
    pub registry_version: u32,
    pub node_index: u32,
    pub epoch: u64,
    /// Opaque batch header bytes, covered by the signature but not interpreted on-chain.
    pub batch_meta: [u8; 32],
    pub batch_root: [u8; 32],
    /// Single-key EVM-Schnorr signature over the batch message.
    pub batch_sig_s: [u8; 32],
    pub batch_commitment_addr: [u8; 20],
    pub slot: u64,
    pub nonces: CommittedNoncePair,
    pub merkle_proof: Vec<[u8; 32]>,
    pub transcript_a: PartialTranscript,
    pub transcript_b: PartialTranscript,
}

impl Evidence for NonceReuseEvidence {
    /// Evidence hash for nonce-slot reuse; also the `PunishmentCase` PDA seed.
    ///
    /// Covers the offence identity only: which node, under which registry version, reused which
    /// committed slot, with which two transcripts. The merkle path and batch signature are
    /// verification witnesses rather than identity, so they are excluded — a given leaf under a signed
    /// root has exactly one path anyway. Transcript hashes are order-normalized so that swapping the
    /// two transcripts cannot open a second case over one offence.
    fn evidence_hash(&self) -> [u8; 32] {
        let (lo, hi) = canonical_pair(
            partial_transcript_hash(&self.transcript_a),
            partial_transcript_hash(&self.transcript_b),
        );
        hashv(&[
            DOMAIN_NONCE_REUSE,
            self.registry_version.to_be_bytes().as_slice(),
            self.node_index.to_be_bytes().as_slice(),
            self.epoch.to_be_bytes().as_slice(),
            self.batch_meta.as_slice(),
            self.batch_root.as_slice(),
            self.slot.to_be_bytes().as_slice(),
            self.nonces.r1_x.as_slice(),
            self.nonces.r1_y.as_slice(),
            self.nonces.r2_x.as_slice(),
            self.nonces.r2_y.as_slice(),
            lo.as_slice(),
            hi.as_slice(),
        ])
        .to_bytes()
    }

    fn offence_kind(&self) -> OffenceKind {
        OffenceKind::NonceReuse
    }

    fn penalty(&self) -> Penalty {
        self.offence_kind().penalty()
    }

    fn registry_version(&self) -> u32 {
        self.registry_version
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UnsolicitedSigningEvidence {
    pub attestation: Attestation,
}

impl Evidence for UnsolicitedSigningEvidence {
    fn evidence_hash(&self) -> [u8; 32] {
        hashv(&[
            DOMAIN_UNSOLICITED_SIGNING,
            attestation_hash(&self.attestation).as_slice(),
        ])
        .to_bytes()
    }

    fn offence_kind(&self) -> OffenceKind {
        OffenceKind::UnsolicitedSigning
    }

    fn penalty(&self) -> Penalty {
        self.offence_kind().penalty()
    }

    fn registry_version(&self) -> u32 {
        self.attestation.payload.registry_version
    }
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

/// Lexicographically order two 32-byte digests so pair-wise evidence is attestation-order
/// independent.
fn canonical_pair(hash_a: [u8; 32], hash_b: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    if hash_a <= hash_b {
        (hash_a, hash_b)
    } else {
        (hash_b, hash_a)
    }
}

/// Hash every payload and signature field of one attestation (integers big-endian).
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