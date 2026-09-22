//! `slash_offence` inactivity: freeze a live node whose liveliness score is below threshold.
use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::{NodePunished, OffenceProven, RegistryUpdated, RegistryVersionReason};
use molpha::state::{InactivityEvidence, Node, NodeStatus, Offence, OffenceKind, PunishmentCase};
use molpha_tests::env::{assert_molpha_error, events, only_event};
use molpha_tests::{pda, scenario::Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn setup() -> (Protocol, Keypair, InactivityEvidence, Pubkey) {
    let mut p = Protocol::start();
    p.register_nodes("unified-inactivity", 4);
    let reporter = p.reporter();
    let evidence = InactivityEvidence {
        registry_version: p.current_version(),
        node_index: 1,
    };
    let node = p.env.registry(evidence.registry_version).nodes[evidence.node_index as usize];
    (p, reporter, evidence, node)
}

fn wrap(evidence: InactivityEvidence) -> Offence {
    Offence::Inactivity(evidence)
}

#[test]
fn inactivity_freezes_without_slashing_and_records_a_unique_case() {
    let (mut p, reporter, evidence, node_pda) = setup();
    let threshold = p.args.liveliness_freeze_threshold;
    p.overwrite_node(&node_pda, |node| node.liveliness_score = threshold - 1);
    let before: Node = p.env.get(&node_pda);
    let vault = p.node_deposit_balance();
    let treasury = p.treasury_balance();
    let now = p.env.unix_timestamp();
    let wrapped = wrap(evidence.clone());
    let hash = wrapped.hash();
    let ix = p.slash_offence_ix(&reporter.pubkey(), wrapped);
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let node: Node = p.env.get(&node_pda);
    assert_eq!(node.status, NodeStatus::Frozen);
    assert_eq!(
        node.frozen_until,
        now + molpha::state::INACTIVITY_FREEZE_SECS
    );
    assert_eq!(node.locked_amount, before.locked_amount);
    assert_eq!(node.claimable_rewards, before.claimable_rewards);
    assert_eq!(p.current_version(), evidence.registry_version + 1);
    assert!(!p
        .env
        .registry(p.current_version())
        .nodes
        .contains(&node_pda));
    assert_eq!(p.node_deposit_balance(), vault);
    assert_eq!(p.treasury_balance(), treasury);
    assert_eq!(
        p.env
            .token_balance(&pda::ata(&reporter.pubkey(), &p.usdc_mint)),
        0
    );
    let record: PunishmentCase = p.env.get(&pda::punishment_case(&hash));
    assert_eq!(record.offence, OffenceKind::Inactivity);
    assert_eq!(record.guilty_count, 1);
    assert_eq!(record.slashed_total, 0);
    assert_eq!(record.bounty_paid, 0);
    let proven: OffenceProven = only_event(&meta);
    assert_eq!(proven.offence, OffenceKind::Inactivity);
    assert_eq!(proven.evidence_hash, hash);
    assert_eq!(proven.slashed_total, 0);
    let version: RegistryUpdated = only_event(&meta);
    assert_eq!(version.new_version, p.current_version());
    assert_eq!(
        version.node_count,
        p.env.registry(p.current_version()).node_count
    );
    assert!(matches!(
        version.reason,
        RegistryVersionReason::NodePunished
    ));
    let punished = events::<NodePunished>(&meta);
    assert_eq!(punished.len(), 1);
    assert_eq!(punished[0].node, node_pda);
    assert_eq!(punished[0].new_status, NodeStatus::Frozen);
    p.assert_solvent();

    // A second node must not collide with the first inactivity case.
    let second = p.env.registry(p.current_version()).nodes[0];
    p.overwrite_node(&second, |node| node.liveliness_score = threshold - 1);
    let evidence = wrap(InactivityEvidence {
        registry_version: p.current_version(),
        node_index: 0,
    });
    assert_ne!(evidence.hash(), hash);
    let ix = p.slash_offence_ix(&reporter.pubkey(), evidence);
    p.env.send_ok(&[ix], &[&reporter]);
    assert_eq!(p.env.get::<Node>(&second).status, NodeStatus::Frozen);
}

#[test]
fn inactivity_rejects_healthy_inactive_and_mismatched_nodes() {
    for invalid in 0..3 {
        let (mut p, reporter, evidence, node_pda) = setup();
        let threshold = p.args.liveliness_freeze_threshold;
        p.overwrite_node(&node_pda, |node| {
            node.liveliness_score = if invalid == 0 {
                threshold
            } else {
                threshold - 1
            };
            if invalid == 1 {
                node.status = NodeStatus::Frozen;
            }
        });
        let version = p.current_version();
        let wrapped = wrap(evidence);
        let hash = wrapped.hash();
        let mut ix = p.slash_offence_ix(&reporter.pubkey(), wrapped);
        let expected = match invalid {
            0 => MolphaError::LivelinessAboveThreshold,
            1 => MolphaError::NodeNotActive,
            _ => {
                ix.accounts.last_mut().unwrap().pubkey = p.env.registry(version).nodes[0];
                MolphaError::InvalidNodeAccount
            }
        };
        let failed = p.env.send_err(&[ix], &[&reporter]);
        assert_molpha_error(&failed, expected);
        assert_eq!(p.current_version(), version);
        assert!(p
            .env
            .svm
            .get_account(&pda::punishment_case(&hash))
            .is_none());
    }
}

#[test]
fn inactivity_rejects_historical_registry_evidence() {
    let (mut p, reporter, evidence, _) = setup();
    p.register_nodes("new-registry", 1);
    let version = p.current_version();
    let ix = p.slash_offence_ix(&reporter.pubkey(), wrap(evidence));
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::RegistryVersionMismatch);
    assert_eq!(p.current_version(), version);
}
