//! `slash_offence` inactivity: freeze a live node whose availability in a finalized epoch fell
//! below that epoch's payout floor. Every case builds the epoch from real tickets.
use molpha::error::MolphaError;
use molpha::events::{NodePunished, OffenceProven, RegistryUpdated, RegistryVersionReason};
use molpha::state::{
    InactivityEvidence, Node, NodeStatus, Offence, OffenceKind, PunishmentCase,
    INACTIVITY_FREEZE_SECS,
};
use molpha_tests::env::{assert_molpha_error, events, only_event};
use molpha_tests::{pda, scenario::Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 4;
const SAMPLES: u16 = 3;

/// Four nodes, quorum 3 plus buffer 2: every round selects every node, so a round that leaves
/// one node out counts it as a selected miss.
fn setup(min_samples: u16) -> (Protocol, Keypair) {
    let mut p = Protocol::builder()
        .min_availability_samples(min_samples)
        .start();
    p.register_nodes("epoch-inactivity", NODE_COUNT);
    let gateway = p.register_gateway();
    (p, gateway)
}

// The failure metadata carries the logs, which is what makes a failing test readable.
#[allow(clippy::result_large_err)]
fn file(p: &mut Protocol, evidence: InactivityEvidence) -> litesvm::types::TransactionResult {
    let reporter = p.reporter();
    let ix = p.slash_offence_ix(&reporter.pubkey(), Offence::Inactivity(evidence));
    p.env.send(&[ix], &[&reporter])
}

#[test]
fn inactivity_freezes_without_slashing_and_records_a_unique_case() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));
    let state = p.env.epoch(epoch);
    let slot = usize::from(p.roster_slot_of(epoch, &node_pda).unwrap());
    assert_eq!(state.signed[slot], 0);
    assert_eq!(state.selected[slot], SAMPLES);
    assert!(state.below_availability_floor(slot));

    let before: Node = p.env.get(&node_pda);
    let vault = p.node_deposit_balance();
    let treasury = p.treasury_balance();
    let version = p.current_version();
    let now = p.env.unix_timestamp();
    let evidence = p.inactivity_evidence(epoch, &node_pda);
    let hash = Offence::Inactivity(evidence.clone()).hash();
    let meta = file(&mut p, evidence).expect("inactivity filing succeeds");

    let node: Node = p.env.get(&node_pda);
    assert_eq!(node.status, NodeStatus::Frozen);
    assert_eq!(node.frozen_until, now + INACTIVITY_FREEZE_SECS);
    assert_eq!(node.locked_amount, before.locked_amount);
    assert_eq!(node.claimable_rewards, before.claimable_rewards);
    assert_eq!(p.current_version(), version + 1);
    assert!(!p
        .env
        .registry(p.current_version())
        .nodes
        .contains(&node_pda));
    assert_eq!(p.node_deposit_balance(), vault);
    assert_eq!(p.treasury_balance(), treasury);

    let record: PunishmentCase = p.env.get(&pda::punishment_case(&hash));
    assert_eq!(record.offence, OffenceKind::Inactivity);
    assert_eq!(record.guilty_count, 1);
    assert_eq!(record.slashed_total, 0);
    assert_eq!(record.bounty_paid, 0);
    let proven: OffenceProven = only_event(&meta);
    assert_eq!(proven.offence, OffenceKind::Inactivity);
    assert_eq!(proven.evidence_hash, hash);
    let registry: RegistryUpdated = only_event(&meta);
    assert!(matches!(
        registry.reason,
        RegistryVersionReason::NodePunished
    ));
    let punished = events::<NodePunished>(&meta);
    assert_eq!(punished.len(), 1);
    assert_eq!(punished[0].node, node_pda);
    assert_eq!(punished[0].new_status, NodeStatus::Frozen);
    p.assert_solvent();
}

#[test]
fn a_node_that_signed_its_share_is_above_the_floor() {
    let (mut p, gateway) = setup(SAMPLES);
    let missing = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &missing, usize::from(SAMPLES));
    let healthy = p.nodes[0].pda;

    let evidence = p.inactivity_evidence(epoch, &healthy);
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::AvailabilityAboveFloor);
    assert_eq!(p.env.get::<Node>(&healthy).status, NodeStatus::Active);
}

/// The sample minimum keeps the floor from firing on noise: a node sampled fewer times keeps its
/// plain share and cannot be frozen, however bad its rate.
#[test]
fn too_few_samples_never_trigger_the_floor() {
    let (mut p, gateway) = setup(SAMPLES + 1);
    let node_pda = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));

    let evidence = p.inactivity_evidence(epoch, &node_pda);
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::AvailabilityAboveFloor);
}

#[test]
fn an_open_epoch_is_not_evidence() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.open_epoch();
    let submitter = p.env.keypair();
    for duty in p.duties_selecting(&node_pda, usize::from(SAMPLES), 0x21) {
        let signed = p.sign_excluding(duty, &[node_pda]);
        p.submit_ticket(&submitter, &gateway, &signed);
    }

    let evidence = p.inactivity_evidence(epoch, &node_pda);
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::EpochNotFinalized);
}

#[test]
fn evidence_must_name_the_node_on_both_the_roster_and_the_registry() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));
    let other = p.nodes[2].pda;

    // Roster slot of another node.
    let mut evidence = p.inactivity_evidence(epoch, &node_pda);
    evidence.roster_slot = p.roster_slot_of(epoch, &other).unwrap();
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);

    // Registry index of another node (the builder then passes that node's account).
    let mut evidence = p.inactivity_evidence(epoch, &node_pda);
    evidence.node_index = u32::from(
        p.env
            .registry(p.current_version())
            .index_of(&other)
            .unwrap(),
    );
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidNodeAccount);
    assert_eq!(p.env.get::<Node>(&node_pda).status, NodeStatus::Active);
}

#[test]
fn inactivity_rejects_historical_registry_evidence() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));
    let evidence = p.inactivity_evidence(epoch, &node_pda);
    p.register_nodes("new-registry", 1);

    let version = p.current_version();
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::RegistryVersionMismatch);
    assert_eq!(p.current_version(), version);
}

/// The case hash covers `(epoch, roster_slot)` only: an early reinstatement moves the node to a
/// new registry version and index, but the same outage cannot be refiled.
#[test]
fn a_node_is_punished_at_most_once_per_epoch() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.freeze_via_inactivity(&gateway, &node_pda);

    let authority = p.authority.insecure_clone();
    let owner = p.nodes[1].owner.pubkey();
    let ix = p.reinstate_node_ix(&authority.pubkey(), &owner);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(p.env.get::<Node>(&node_pda).status, NodeStatus::Active);

    let evidence = p.inactivity_evidence(epoch, &node_pda);
    let failed = file(&mut p, evidence).unwrap_err();
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|line| line.contains("already in use")),
        "expected the existing case to block a refiling"
    );
    assert_eq!(p.env.get::<Node>(&node_pda).status, NodeStatus::Active);
}

/// A freeze that overlapped the next epoch already covers the misses it caused there.
#[test]
fn a_freeze_overlapping_an_epoch_blocks_a_second_freeze_for_it() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    p.freeze_via_inactivity(&gateway, &node_pda);

    let authority = p.authority.insecure_clone();
    let owner = p.nodes[1].owner.pubkey();
    let ix = p.reinstate_node_ix(&authority.pubkey(), &owner);
    p.env.send_ok(&[ix], &[&authority]);

    let next = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));
    assert!(p.env.get::<Node>(&node_pda).frozen_until > p.env.epoch(next).start);
    let evidence = p.inactivity_evidence(next, &node_pda);
    let failed = file(&mut p, evidence).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InactivityAlreadyPunished);
}

/// Closing waits one epoch length past the ticket deadline so evidence can be filed; once the
/// account is gone, so is the evidence.
#[test]
fn a_closed_epoch_is_no_longer_evidence() {
    let (mut p, gateway) = setup(SAMPLES);
    let node_pda = p.nodes[1].pda;
    let epoch = p.run_epoch_missing(&gateway, &node_pda, usize::from(SAMPLES));
    let evidence = p.inactivity_evidence(epoch, &node_pda);

    p.credit_all(epoch);
    let state = p.env.epoch(epoch);
    p.env.warp_to(Protocol::close_deadline(&state));
    let open = p.open_epoch();
    p.env.send_ok(&[p.close_epoch_ix(epoch, open)], &[]);

    assert!(file(&mut p, evidence).is_err());
    assert_eq!(p.env.get::<Node>(&node_pda).status, NodeStatus::Active);
}
