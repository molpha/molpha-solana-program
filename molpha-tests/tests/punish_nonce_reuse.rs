//! `slash_nonce_reuse`: two verifying partials against one batch-committed nonce pair.

use molpha::error::MolphaError;
use molpha::events::{NodePunished, NonceReuseProven};
use molpha::state::{
    CommittedNoncePair, Evidence, MembershipAction, Node, NodeStatus, NonceReuseEvidence,
    OffenceKind, PartialTranscript, PunishmentCase, MAX_COMMITMENT_PROOF_DEPTH,
};
use molpha_tests::crypto::{self, CommitmentBatchTree, CommittedSlot, NodeKey};
use molpha_tests::env::{assert_molpha_error, events, only_event, PACKET_DATA_SIZE};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const EPOCH: u64 = 42;
const BATCH_META: [u8; 32] = [0x77; 32];
/// The slot whose nonce pair gets consumed twice.
const REUSED_SLOT: u64 = 5;
const SLOTS_PER_BATCH: u64 = 8;

/// Two challenge pairs the node evaluates against one commitment. Distinct `(b, c)` is the offence.
const B_A: [u8; 32] = [0x03; 32];
const C_A: [u8; 32] = [0x05; 32];
const B_B: [u8; 32] = [0x07; 32];
const C_B: [u8; 32] = [0x11; 32];

/// Solana's per-transaction maximum, which the harness requests for every transaction.
const MAX_COMPUTE_UNITS: u64 = 1_400_000;

/// Regression ceiling. A filing measures ~690k CU and is dominated by the fixed cost every slash
/// pays — allocating and rotating two 8 KiB registry accounts, plus two token transfers. The five
/// `secp256k1_recover` syscalls that verify the batch signature and both partial equations are
/// ~25k each, so the cryptography is a sixth of the bill and proof depth barely moves it.
const NONCE_REUSE_CU_CEILING: u64 = 800_000;

/// Deepest merkle proof whose filing still fits a legacy packet, asserted exactly.
///
/// Args are fixed-size apart from the proof, so each level of batch depth costs exactly 32 bytes.
/// Deeper batches stay filable through a v0 transaction with an address lookup table.
const MAX_LEGACY_PROOF_DEPTH: usize = 5;

struct Case {
    p: Protocol,
    reporter: Keypair,
    node_index: u32,
    node_key: NodeKey,
    batch: CommitmentBatchTree,
    leaf_index: usize,
    registry_version: u32,
}

impl Case {
    fn slot(&self) -> &CommittedSlot {
        &self.batch.slots[self.leaf_index]
    }

    fn nonces(&self) -> CommittedNoncePair {
        let slot = self.slot();
        CommittedNoncePair {
            r1_x: slot.r1.x,
            r1_y: slot.r1.y,
            r2_x: slot.r2.x,
            r2_y: slot.r2.y,
        }
    }

    fn transcript(&self, b: [u8; 32], c: [u8; 32]) -> PartialTranscript {
        PartialTranscript {
            b,
            c,
            s: crypto::forge_partial_transcript(self.slot(), &self.node_key, &b, &c),
        }
    }

    fn guilty_pda(&self) -> anchor_lang::prelude::Pubkey {
        self.p.env.registry(self.registry_version).nodes[self.node_index as usize]
    }
}

/// A node that committed a batch of nonce pairs and then signed twice against one of them.
fn case() -> Case {
    case_with_batch(SLOTS_PER_BATCH)
}

fn case_with_batch(slots: u64) -> Case {
    let mut p = Protocol::start();
    p.register_nodes("nonce-reuse", NODE_COUNT);
    let reporter = p.reporter();
    let registry_version = p.current_version();

    // The offender is whichever node sits at registry index 0.
    let registry = p.env.registry(registry_version);
    let node_index = 0u32;
    let pda_at_index = registry.nodes[node_index as usize];
    let node_key = p
        .nodes
        .iter()
        .find(|n| n.pda == pda_at_index)
        .expect("registry entry is a registered node")
        .key
        .clone();

    let batch = CommitmentBatchTree::derive("nonce-reuse-batch", 0, slots);
    let leaf_index = batch
        .slots
        .iter()
        .position(|s| s.slot == REUSED_SLOT.min(slots - 1))
        .expect("reused slot is in the batch");

    Case {
        p,
        reporter,
        node_index,
        node_key,
        batch,
        leaf_index,
        registry_version,
    }
}

fn args_for(case: &Case) -> NonceReuseEvidence {
    args_with_transcripts(case, case.transcript(B_A, C_A), case.transcript(B_B, C_B))
}

fn args_with_transcripts(
    case: &Case,
    transcript_a: PartialTranscript,
    transcript_b: PartialTranscript,
) -> NonceReuseEvidence {
    let batch_root = case.batch.root();
    let signature = crypto::sign_commitment_batch(
        &case.node_key,
        &molpha::ID,
        case.registry_version,
        case.node_index,
        EPOCH,
        &BATCH_META,
        &batch_root,
    );
    let nonces = case.nonces();
    NonceReuseEvidence {
        registry_version: case.registry_version,
        node_index: case.node_index,
        epoch: EPOCH,
        batch_meta: BATCH_META,
        batch_root,
        batch_sig_s: signature.agg_sig_s,
        batch_commitment_addr: signature.commitment,
        slot: case.slot().slot,
        nonces,
        merkle_proof: case.batch.proof(case.leaf_index),
        transcript_a,
        transcript_b,
    }
}

fn resealed(args: NonceReuseEvidence) -> NonceReuseEvidence {
    args
}

fn file(case: &mut Case, args: NonceReuseEvidence) -> litesvm::types::TransactionMetadata {
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_nonce_reuse_ix(&reporter.pubkey(), args);
    case.p.env.send_ok(&[ix], &[&reporter])
}

fn file_err(
    case: &mut Case,
    args: NonceReuseEvidence,
) -> litesvm::types::FailedTransactionMetadata {
    let reporter = case.reporter.insecure_clone();
    let ix = case.p.slash_nonce_reuse_ix(&reporter.pubkey(), args);
    case.p.env.send_err(&[ix], &[&reporter])
}

#[test]
fn a_proven_nonce_reuse_takes_the_whole_deposit_tombstones_and_evicts_the_node() {
    let mut case = case();
    let args = args_for(&case);
    let guilty = case.guilty_pda();

    let locked = case.p.args.minimum_node_deposit;
    // Unlike commitment reuse, reusing a committed nonce is key-compromise-equivalent.
    let penalty = OffenceKind::NonceReuse.penalty();
    assert_eq!(penalty.slash_bps, 10_000);
    assert_eq!(penalty.membership, MembershipAction::Tombstone);
    let expected_bounty =
        (locked as u128 * case.p.args.challenger_bounty_bps as u128 / 10_000) as u64;
    let treasury_before = case.p.treasury_balance();
    let new_version = case.p.current_version() + 1;

    let meta = file(&mut case, args.clone());
    println!(
        "punish_nonce_reuse: depth {} -> {} CU",
        args.merkle_proof.len(),
        meta.compute_units_consumed
    );
    assert!(
        meta.compute_units_consumed < NONCE_REUSE_CU_CEILING,
        "filing got more expensive: {} CU",
        meta.compute_units_consumed
    );

    let account: Node = case.p.env.get(&guilty);
    assert_eq!(account.status, NodeStatus::Tombstoned);
    assert_eq!(account.locked_amount, 0, "the full deposit is slashed");
    assert_eq!(
        account.withdrawable_slot,
        case.p
            .env
            .slot()
            .saturating_add(case.p.args.withdrawal_cooldown_slots)
    );
    for node in case.p.node_pdas() {
        if node != guilty {
            let other: Node = case.p.env.get(&node);
            assert_eq!(other.status, NodeStatus::Active);
            assert_eq!(other.locked_amount, locked);
        }
    }

    let after = case.p.env.registry(new_version);
    assert_eq!(after.node_count as usize, NODE_COUNT - 1);
    assert!(!after.nodes.contains(&guilty));

    let reporter_usdc = pda::ata(&case.reporter.pubkey(), &case.p.usdc_mint);
    assert_eq!(case.p.env.token_balance(&reporter_usdc), expected_bounty);
    assert_eq!(
        case.p.treasury_balance(),
        treasury_before + locked - expected_bounty
    );
    assert_eq!(
        case.p.node_deposit_balance(),
        case.p.total_locked_deposits(),
        "the vault matches what is still owed to nodes"
    );
    case.p.assert_solvent();

    let record: PunishmentCase = case.p.env.get(&pda::punishment_case(&args.evidence_hash()));
    assert_eq!(record.offence, OffenceKind::NonceReuse);
    assert_eq!(record.guilty_count, 1);
    assert_eq!(record.slashed_total, locked);
    assert_eq!(record.bounty_paid, expected_bounty);
    assert_eq!(record.registry_version, case.registry_version);

    let proven: NonceReuseProven = only_event(&meta);
    assert_eq!(proven.node_index, case.node_index);
    assert_eq!(proven.epoch, EPOCH);
    assert_eq!(proven.slot, case.slot().slot);
    assert_eq!(proven.batch_root, args.batch_root);
    assert_eq!(proven.r1_x, args.nonces.r1_x);
    assert_eq!(proven.guilty_count, 1);
    assert_eq!(proven.slashed_total, locked);
    assert_eq!(events::<NodePunished>(&meta).len(), 1);
}

/// A tombstoned node keeps nothing to slash, so a second filing has nothing to do.
#[test]
fn a_second_filing_against_a_tombstoned_node_finds_nothing_to_punish() {
    let mut case = case();
    let args = args_for(&case);
    file(&mut case, args);

    let fresh = args_with_transcripts(
        &case,
        case.transcript([0x21; 32], [0x23; 32]),
        case.transcript([0x27; 32], [0x29; 32]),
    );
    let err = file_err(&mut case, fresh);
    assert_molpha_error(&err, MolphaError::NothingToPunish);
}

#[test]
fn the_same_evidence_cannot_be_filed_twice() {
    let mut case = case();
    let args = args_for(&case);
    file(&mut case, args.clone());

    // The punishment case PDA is seeded by the evidence hash, so the replay collides on `init`.
    let err = file_err(&mut case, args);
    assert!(
        err.meta.logs.iter().any(|l| l.contains("already in use")),
        "logs:\n{}",
        err.meta.logs.join("\n")
    );
}

/// Swapping the two transcripts is the same offence, so it must not open a second case.
#[test]
fn transcript_order_does_not_change_the_evidence_hash() {
    let case = case();
    let forward = args_for(&case);
    let reversed =
        args_with_transcripts(&case, case.transcript(B_B, C_B), case.transcript(B_A, C_A));
    assert_eq!(forward.evidence_hash(), reversed.evidence_hash());
}

#[test]
fn one_challenge_pair_used_once_is_not_reuse() {
    let mut case = case();
    let transcript = case.transcript(B_A, C_A);
    // Same (b, c) twice: the equation fixes `s`, so this is one evaluation reported twice.
    let args = resealed(args_with_transcripts(&case, transcript, transcript));
    let err = file_err(&mut case, args);
    assert_molpha_error(&err, MolphaError::TranscriptContextsEqual);
}

#[test]
fn a_transcript_that_does_not_satisfy_the_equation_proves_nothing() {
    let mut case = case();
    let mut args = args_for(&case);
    args.transcript_b.s[31] ^= 1;
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidPartialSignature);
}

/// A filer holding one observed transcript cannot invent the second one without the node's secrets.
#[test]
fn a_transcript_forged_against_another_nodes_key_does_not_verify() {
    let mut case = case();
    let other = case
        .p
        .nodes
        .iter()
        .find(|n| n.pda != case.guilty_pda())
        .expect("another registered node")
        .key
        .clone();
    let b = [0x31; 32];
    let c = [0x37; 32];
    let forged = PartialTranscript {
        b,
        c,
        s: crypto::forge_partial_transcript(case.slot(), &other, &b, &c),
    };
    let args = resealed(args_with_transcripts(
        &case,
        case.transcript(B_A, C_A),
        forged,
    ));
    let err = file_err(&mut case, args);
    assert_molpha_error(&err, MolphaError::InvalidPartialSignature);
}

#[test]
fn non_canonical_and_zero_scalars_are_rejected() {
    // n itself, and zero: both outside the canonical scalar range.
    const ORDER: [u8; 32] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36,
        0x41, 0x41,
    ];
    for bad in [ORDER, [0u8; 32]] {
        let mut case = case();
        let mut args = args_for(&case);
        args.transcript_a.b = bad;
        let err = file_err(&mut case, resealed(args));
        assert_molpha_error(&err, MolphaError::NonCanonicalScalar);

        let mut args = args_for(&case);
        args.transcript_b.s = bad;
        let err = file_err(&mut case, resealed(args));
        assert_molpha_error(&err, MolphaError::NonCanonicalScalar);
    }
}

/// A node that commits garbage instead of a curve point produces unevaluable evidence, not a
/// slash. The bad point has to be inside the signed leaf, otherwise the merkle check rejects first.
#[test]
fn a_committed_nonce_that_is_not_a_curve_point_is_rejected() {
    let mut case = case_with_batch(1);
    let mut args = args_for(&case);
    args.nonces.r1_y[31] ^= 1;

    // Single-leaf batch: the root is the leaf, so re-sign a root that commits the bad point.
    args.batch_root = crypto::commitment_leaf(
        args.slot,
        &args.nonces.r1_x,
        &args.nonces.r1_y,
        &args.nonces.r2_x,
        &args.nonces.r2_y,
    );
    let signature = crypto::sign_commitment_batch(
        &case.node_key,
        &molpha::ID,
        case.registry_version,
        case.node_index,
        EPOCH,
        &BATCH_META,
        &args.batch_root,
    );
    args.batch_sig_s = signature.agg_sig_s;
    args.batch_commitment_addr = signature.commitment;

    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidNoncePoint);
}

#[test]
fn nonces_outside_the_committed_leaf_do_not_reach_the_root() {
    let mut case = case();
    let other_slot = CommittedSlot::derive("uncommitted", 999);
    let mut args = args_for(&case);
    args.nonces = CommittedNoncePair {
        r1_x: other_slot.r1.x,
        r1_y: other_slot.r1.y,
        r2_x: other_slot.r2.x,
        r2_y: other_slot.r2.y,
    };
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidMerkleProof);
}

#[test]
fn a_leaf_claimed_under_the_wrong_slot_does_not_reach_the_root() {
    let mut case = case();
    let mut args = args_for(&case);
    args.slot += 1;
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidMerkleProof);
}

#[test]
fn a_tampered_merkle_path_is_rejected() {
    let mut case = case();
    let mut args = args_for(&case);
    args.merkle_proof[0][0] ^= 1;
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidMerkleProof);
}

#[test]
fn a_proof_deeper_than_the_cap_is_refused_before_any_hashing() {
    let mut case = case();
    let mut args = args_for(&case);
    args.merkle_proof = vec![[0u8; 32]; MAX_COMMITMENT_PROOF_DEPTH + 1];
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::MerkleProofTooDeep);
}

#[test]
fn a_batch_signature_that_does_not_cover_the_filed_header_is_rejected() {
    // Every bound header field must break the signature when changed.
    for mutate in [
        (|args: &mut NonceReuseEvidence| args.epoch += 1) as fn(&mut NonceReuseEvidence),
        |args: &mut NonceReuseEvidence| args.batch_meta[0] ^= 1,
        |args: &mut NonceReuseEvidence| args.batch_sig_s[31] ^= 1,
        |args: &mut NonceReuseEvidence| args.batch_commitment_addr[0] ^= 1,
    ] {
        let mut case = case();
        let mut args = args_for(&case);
        mutate(&mut args);
        let err = file_err(&mut case, resealed(args));
        assert_molpha_error(&err, MolphaError::InvalidBatchSignature);
    }
}

/// A batch root nobody signed cannot be laundered by rebuilding a consistent proof around it.
#[test]
fn an_unsigned_batch_root_is_rejected() {
    let mut case = case();
    let other = CommitmentBatchTree::derive("some-other-batch", 0, SLOTS_PER_BATCH);
    let mut args = args_for(&case);
    args.batch_root = other.root();
    args.merkle_proof = other.proof(case.leaf_index);
    args.nonces = CommittedNoncePair {
        r1_x: other.slots[case.leaf_index].r1.x,
        r1_y: other.slots[case.leaf_index].r1.y,
        r2_x: other.slots[case.leaf_index].r2.x,
        r2_y: other.slots[case.leaf_index].r2.y,
    };
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidBatchSignature);
}

/// An index past `node_count` names no registry entry, so the lookup refuses it. Built by hand: the
/// scenario builder derives its accounts from the registry and cannot address a slot that is empty.
#[test]
fn a_node_index_outside_the_registry_is_rejected() {
    use anchor_lang::{InstructionData, ToAccountMetas};

    let mut case = case();
    let mut args = args_for(&case);
    args.node_index = 250;
    let args = resealed(args);

    let reporter = case.reporter.insecure_clone();
    let mut accounts = molpha::accounts::SlashNonceReuse {
        reporter: reporter.pubkey(),
        protocol_config: pda::protocol_config(),
        registry_state: pda::registry_state(),
        evidence_registry: pda::registry(args.registry_version),
        current_registry: pda::registry(case.p.current_version()),
        new_registry: pda::registry(case.p.current_version() + 1),
        punishment_case: pda::punishment_case(&args.evidence_hash()),
        node_deposit_vault: case.p.node_deposit_vault(),
        treasury: case.p.treasury(),
        reporter_usdc: pda::ata(&reporter.pubkey(), &case.p.usdc_mint),
        token_program: pda::TOKEN_PROGRAM_ID,
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    accounts.extend(molpha_tests::env::writable_metas(vec![case.guilty_pda()]));

    let ix = anchor_lang::solana_program::instruction::Instruction {
        program_id: molpha::ID,
        accounts,
        data: molpha::instruction::SlashNonceReuse { evidence: args }.data(),
    };
    let err = case.p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&err, MolphaError::InvalidNodeIndex);
}

/// The batch signature binds `node_index`, so pointing a real batch at a different node fails.
#[test]
fn evidence_cannot_be_retargeted_at_another_node() {
    let mut case = case();
    let mut args = args_for(&case);
    args.node_index = 1;
    let err = file_err(&mut case, resealed(args));
    assert_molpha_error(&err, MolphaError::InvalidBatchSignature);
}

/// A single-leaf batch is the degenerate tree: the root is the leaf and the proof is empty.
#[test]
fn a_single_slot_batch_files_with_an_empty_proof() {
    let mut case = case_with_batch(1);
    let args = args_for(&case);
    assert!(args.merkle_proof.is_empty());
    file(&mut case, args);
    assert_eq!(
        case.p.env.get::<Node>(&case.guilty_pda()).status,
        NodeStatus::Tombstoned
    );
}

/// The proof is the only variable-length part of the filing; pin what a legacy packet can carry.
#[test]
fn proof_depth_drives_the_wire_size() {
    let case = case();
    let reporter = case.reporter.pubkey();
    let base = args_for(&case);

    let mut largest_fitting = None;
    for depth in 0..=MAX_COMMITMENT_PROOF_DEPTH {
        let mut args = base.clone();
        args.merkle_proof = vec![[0u8; 32]; depth];
        let ix = case.p.slash_nonce_reuse_ix(&reporter, resealed(args));
        let size = case.p.env.transaction_size_as(&reporter, &[ix]);
        println!("punish_nonce_reuse: proof depth {depth:>2} -> {size} bytes legacy");
        if size <= PACKET_DATA_SIZE {
            largest_fitting = Some(depth);
        }
    }

    assert_eq!(
        largest_fitting,
        Some(MAX_LEGACY_PROOF_DEPTH),
        "the batch depth a legacy nonce-reuse filing can carry changed"
    );
}

#[test]
fn the_deepest_accepted_proof_still_lands_within_the_compute_budget() {
    // 2^14 slots: the deepest batch the instruction accepts.
    let mut case = case_with_batch(1 << MAX_COMMITMENT_PROOF_DEPTH);
    let args = args_for(&case);
    assert_eq!(args.merkle_proof.len(), MAX_COMMITMENT_PROOF_DEPTH);

    let meta = file(&mut case, args);
    println!(
        "punish_nonce_reuse: depth {MAX_COMMITMENT_PROOF_DEPTH} -> {} CU",
        meta.compute_units_consumed
    );
    assert!(meta.compute_units_consumed < MAX_COMPUTE_UNITS);
    assert_eq!(
        case.p.env.get::<Node>(&case.guilty_pda()).status,
        NodeStatus::Tombstoned
    );
}
