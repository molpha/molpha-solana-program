//! Liveliness score decay, recovery, and permissionless inactivity freezing.

use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::NodeFrozenForInactivity;
use molpha::state::{
    Node, NodeStatus, OffenceKind, RoundReceiptRecord, RoundRecordStatus,
    LIVELINESS_PENALTY_PER_MISS, LIVELINESS_RECOVERY_PER_SIGN, LIVELINESS_SCORE_MAX,
};
use molpha_tests::env::{assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{bitmap_difference, Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 4;
const FLOAT: u64 = 500_000_000;

struct Fixture {
    p: Protocol,
    gateway: Keypair,
}

fn fixture() -> Fixture {
    let mut p = Protocol::start();
    p.register_nodes("liveliness", NODE_COUNT);
    let gateway = p.register_gateway(FLOAT);
    Fixture { p, gateway }
}

fn duty() -> Duty {
    Duty::new(NODE_COUNT as u32, SIGS)
}

impl Fixture {
    fn settle_x402(&mut self, signed: &SignedAttestation, payer_id: [u8; 32]) -> Pubkey {
        let args = self.p.x402_args(signed, payer_id);
        let gateway_pda = pda::gateway(&self.gateway.pubkey());
        let record = pda::x402_round(
            &args.payer_id,
            &gateway_pda,
            &args.source_id,
            args.signatures_required,
            args.canonical_timestamp,
        );
        let ix = self.p.settle_x402_round_ix(&self.gateway.pubkey(), args);
        let gateway = self.gateway.insecure_clone();
        self.p.env.send_ok(&[ix], &[&gateway]);
        record
    }

    fn partial_round(&mut self, payer_id: [u8; 32]) -> (Pubkey, SignedAttestation) {
        let duty = duty();
        let selection = self.p.selection_bits(&duty);
        let signed = self
            .p
            .sign_duty_with_bits(duty, &selection[..SIGS as usize], None);
        let record = self.settle_x402(&signed, payer_id);
        (record, signed)
    }

    fn missed_nodes(&self, record: &Pubkey) -> Vec<Pubkey> {
        let record: RoundReceiptRecord = self.p.env.get(record);
        let missed = bitmap_difference(&record.selection_bitmap, &record.signers_bitmap);
        self.p
            .node_pdas_for_bitmap(record.registry_version, &missed)
    }

    fn finalize(&mut self, record: &Pubkey, signers: &[Pubkey]) {
        self.p
            .env
            .advance_slots(self.p.args.dispute_window_slots + 1);
        let caller = self.p.env.keypair();
        let ix = self.p.finalize_round_ix(&caller.pubkey(), record, signers);
        self.p.env.send_ok(&[ix], &[&caller]);
    }
}

#[test]
fn partial_signing_penalizes_misses_and_recovers_signers_with_a_cap() {
    let mut f = fixture();
    let (record, signed) = f.partial_round([0x01; 32]);
    let missed = f.missed_nodes(&record);
    assert_eq!(missed.len(), f.p.args.redundancy_buffer as usize);

    let recovering = signed.signer_pdas[0];
    f.p.overwrite_node(&recovering, |node| {
        node.liveliness_score = LIVELINESS_SCORE_MAX - 50
    });

    f.finalize(&record, &signed.signer_pdas);

    for missed_node in missed {
        assert_eq!(
            f.p.env.get::<Node>(&missed_node).liveliness_score,
            LIVELINESS_SCORE_MAX - LIVELINESS_PENALTY_PER_MISS
        );
    }
    for signer in signed.signer_pdas {
        assert_eq!(
            f.p.env.get::<Node>(&signer).liveliness_score,
            LIVELINESS_SCORE_MAX
        );
    }
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
}

#[test]
fn finalize_rejects_missing_wrong_and_reordered_missed_accounts() {
    let mut f = fixture();
    let (record, signed) = f.partial_round([0x02; 32]);
    let missed = f.missed_nodes(&record);
    f.p.env.advance_slots(f.p.args.dispute_window_slots + 1);
    let caller = f.p.env.keypair();

    let mut missing =
        f.p.finalize_round_ix(&caller.pubkey(), &record, &signed.signer_pdas);
    let missing_pos = missing
        .accounts
        .iter()
        .position(|meta| meta.pubkey == missed[1])
        .unwrap();
    missing.accounts.remove(missing_pos);
    let failed = f.p.env.send_err(&[missing], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidMissedNodeAccount);

    let mut wrong =
        f.p.finalize_round_ix(&caller.pubkey(), &record, &signed.signer_pdas);
    let wrong_pos = wrong
        .accounts
        .iter()
        .position(|meta| meta.pubkey == missed[0])
        .unwrap();
    let outsider =
        f.p.node_pdas()
            .into_iter()
            .find(|node| !signed.signer_pdas.contains(node) && !missed.contains(node))
            .unwrap();
    wrong.accounts[wrong_pos].pubkey = outsider;
    let failed = f.p.env.send_err(&[wrong], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidMissedNodeAccount);

    let mut reordered =
        f.p.finalize_round_ix(&caller.pubkey(), &record, &signed.signer_pdas);
    let first = reordered
        .accounts
        .iter()
        .position(|meta| meta.pubkey == missed[0])
        .unwrap();
    let second = reordered
        .accounts
        .iter()
        .position(|meta| meta.pubkey == missed[1])
        .unwrap();
    reordered.accounts.swap(first, second);
    let failed = f.p.env.send_err(&[reordered], &[&caller]);
    assert_molpha_error(&failed, MolphaError::InvalidMissedNodeAccount);
}

#[test]
fn validating_a_genuine_partial_round_also_applies_liveliness_changes() {
    let mut f = fixture();
    let (record, signed) = f.partial_round([0x03; 32]);
    let missed = f.missed_nodes(&record);
    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &signed.signer_pdas);
    f.p.env.send_ok(&[ix], &[&challenger]);

    for node in missed {
        assert_eq!(
            f.p.env.get::<Node>(&node).liveliness_score,
            LIVELINESS_SCORE_MAX - LIVELINESS_PENALTY_PER_MISS
        );
    }
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Finalized
    );
}

#[test]
fn voiding_an_invalid_round_does_not_change_liveliness_scores() {
    let mut f = fixture();
    let duty = duty();
    let selection = f.p.selection_bits(&duty);
    let fabricated =
        f.p.sign_duty_with_bits(duty, &selection[..SIGS as usize], None)
            .with_tampered_signature();
    let record = f.settle_x402(&fabricated, [0x04; 32]);
    let challenger = f.p.env.keypair();
    let ix =
        f.p.validate_round_ix(&challenger.pubkey(), &record, None, &fabricated.signer_pdas);
    f.p.env.send_ok(&[ix], &[&challenger]);

    for node in f.p.node_pdas() {
        assert_eq!(
            f.p.env.get::<Node>(&node).liveliness_score,
            LIVELINESS_SCORE_MAX
        );
    }
    assert_eq!(
        f.p.env.get::<RoundReceiptRecord>(&record).status,
        RoundRecordStatus::Voided
    );
}

#[test]
fn a_successful_signature_gradually_recovers_a_low_score() {
    let mut f = fixture();
    let signed = f.p.sign_duty(duty());
    let recovering = signed.signer_pdas[0];
    f.p.overwrite_node(&recovering, |node| node.liveliness_score = 1_000);
    let record = f.settle_x402(&signed, [0x05; 32]);
    f.finalize(&record, &signed.signer_pdas);

    assert_eq!(
        f.p.env.get::<Node>(&recovering).liveliness_score,
        1_000 + LIVELINESS_RECOVERY_PER_SIGN
    );
}

#[test]
fn a_below_threshold_node_can_be_frozen_permissionlessly_and_evicted() {
    let mut p = Protocol::start();
    p.register_nodes("freeze-liveliness", 4);
    let index = 1usize;
    let node_pda = p.nodes[index].pda;
    let owner = p.nodes[index].owner.pubkey();
    let score = p.args.liveliness_freeze_threshold - 1;
    p.overwrite_node(&node_pda, |node| node.liveliness_score = score);

    let caller = p.env.keypair();
    let before = p.env.unix_timestamp();
    let new_version = p.current_version() + 1;
    let ix = p.freeze_unresponsive_node_ix(&caller.pubkey(), &owner);
    let meta = p.env.send_ok(&[ix], &[&caller]);

    let node: Node = p.env.get(&node_pda);
    assert_eq!(node.status, NodeStatus::Frozen);
    assert_eq!(
        node.frozen_until,
        before + OffenceKind::Inactivity.penalty().freeze_secs
    );
    assert_eq!(p.current_version(), new_version);
    let registry = p.env.registry(new_version);
    assert_eq!(registry.node_count, 3);
    assert!(!registry.nodes.contains(&node_pda));

    let event: NodeFrozenForInactivity = only_event(&meta);
    assert_eq!(event.node, node_pda);
    assert_eq!(event.owner, owner);
    assert_eq!(event.score, score);
    assert_eq!(event.frozen_until, node.frozen_until);
    assert_eq!(event.new_registry_version, new_version);
}

#[test]
fn freeze_requires_a_score_strictly_below_threshold_and_an_active_node() {
    let mut p = Protocol::start();
    p.register_nodes("freeze-guards", 4);
    let node_pda = p.nodes[0].pda;
    let owner = p.nodes[0].owner.pubkey();
    let threshold = p.args.liveliness_freeze_threshold;
    p.overwrite_node(&node_pda, |node| node.liveliness_score = threshold);
    let caller = p.env.keypair();
    let ix = p.freeze_unresponsive_node_ix(&caller.pubkey(), &owner);
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::LivelinessAboveThreshold);

    let node_owner = p.nodes[0].owner.insecure_clone();
    let ix = p.deactivate_node_ix(0, &node_owner.pubkey());
    p.env.send_ok(&[ix], &[&node_owner]);
    let ix = p.freeze_unresponsive_node_ix(&caller.pubkey(), &owner);
    let failed = p.env.send_err(&[ix], &[&caller]);
    assert_molpha_error(&failed, MolphaError::NodeNotActive);
}
