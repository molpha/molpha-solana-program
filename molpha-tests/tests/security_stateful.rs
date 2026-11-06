//! Real SBF lifecycle sequences checked against a separate ledger. Proptest shrinks action lists.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

use anchor_lang::prelude::Pubkey;
use anchor_lang::Discriminator;
use molpha::error::MolphaError;
use molpha::instructions::UpdateConfigArgs;
use molpha::state::{
    EquivocationEvidence, Evidence, Node, NodeStatus, PunishmentCase, INACTIVITY_FREEZE_SECS,
};
use molpha_tests::crypto;
use molpha_tests::env::{assert_molpha_error, TestEnv};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use molpha_tests::security::{fee_split, payout, AccountSnapshot, InputRng};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, RngSeed, TestRunner};
use solana_signer::Signer;

#[derive(Default)]
struct EpochModel {
    pool: u64,
    paid: u64,
    tickets: u32,
    selected: BTreeMap<Pubkey, u64>,
    signed: BTreeMap<Pubkey, u64>,
    weights: BTreeMap<Pubkey, u64>,
    credited: BTreeSet<Pubkey>,
    finalized: bool,
}

impl EpochModel {
    fn below_floor(&self, node: &Pubkey) -> bool {
        let selected = u128::from(*self.selected.get(node).unwrap_or(&0));
        let signed = u128::from(*self.signed.get(node).unwrap_or(&0));
        let all_selected: u128 = self.selected.values().map(|v| u128::from(*v)).sum();
        let all_signed: u128 = self.signed.values().map(|v| u128::from(*v)).sum();
        selected >= 3 && signed * all_selected * 10_000 < 8_000 * all_signed * selected
    }
}

struct NodeModel {
    principal: u64,
    rewards: u64,
    status: NodeStatus,
    frozen_until: i64,
}

#[derive(Default)]
struct Ledger {
    treasury: u64,
    reserve: u64,
    registry_version: u32,
    epochs: BTreeMap<u64, EpochModel>,
    nodes: BTreeMap<Pubkey, NodeModel>,
    cases: BTreeSet<(u64, Pubkey)>,
    punishment_accounts: BTreeSet<Pubkey>,
}

impl Ledger {
    fn registered(&mut self, p: &Protocol) {
        let key = p.nodes.last().unwrap().pda;
        self.nodes.insert(
            key,
            NodeModel {
                principal: p.args.minimum_node_deposit,
                rewards: 0,
                status: NodeStatus::Active,
                frozen_until: 0,
            },
        );
    }

    fn assert_matches(&self, p: &Protocol) {
        p.assert_solvent();
        assert_eq!(
            p.current_version(),
            self.registry_version,
            "registry version"
        );
        assert_eq!(p.treasury_balance(), self.treasury, "model treasury");
        assert_eq!(p.config().protocol_reserved, self.reserve, "model reserve");
        let rewards: u64 = self.nodes.values().map(|n| n.rewards).sum();
        assert_eq!(p.reward_liability(), rewards, "model reward liability");
        let pools: u64 = self.epochs.values().map(|e| e.pool - e.paid).sum();
        assert_eq!(p.config().pool_liability, pools, "model pool liability");
        let principal: u64 = self.nodes.values().map(|n| n.principal).sum();
        assert_eq!(
            p.node_deposit_balance(),
            principal,
            "model principal custody"
        );
        for (key, model) in &self.nodes {
            let actual: Node = p.env.get(key);
            assert_eq!(
                (
                    actual.locked_amount,
                    actual.claimable_rewards,
                    actual.status
                ),
                (model.principal, model.rewards, model.status),
                "model node {key}"
            );
            assert_eq!(
                p.env.registry(p.current_version()).index_of(key).is_some(),
                model.status == NodeStatus::Active,
                "active registry membership for {key}"
            );
        }
        for (index, model) in &self.epochs {
            let actual = p.env.epoch(*index);
            assert_eq!(
                (
                    actual.pool,
                    actual.paid,
                    actual.tickets,
                    actual.is_finalized()
                ),
                (model.pool, model.paid, model.tickets, model.finalized),
                "model epoch {index}"
            );
            assert_eq!(usize::from(actual.credited_count), model.credited.len());
            for (key, selected) in &model.selected {
                let slot = usize::from(p.roster_slot_of(*index, key).unwrap());
                assert_eq!(u64::from(actual.selected[slot]), *selected);
                assert_eq!(
                    u64::from(actual.signed[slot]),
                    *model.signed.get(key).unwrap_or(&0)
                );
            }
            if model.finalized {
                assert_eq!(actual.weight_total, model.weights.values().sum::<u64>());
                for (key, weight) in &model.weights {
                    let slot = usize::from(p.roster_slot_of(*index, key).unwrap());
                    assert_eq!(actual.weight(slot), *weight);
                    assert_eq!(actual.is_credited(slot), model.credited.contains(key));
                }
            }
        }
        let actual_cases: BTreeSet<_> = p
            .env
            .svm
            .get_program_accounts(&molpha::ID)
            .into_iter()
            .filter(|(_, account)| {
                account.lamports > 0
                    && account.data.len() == PunishmentCase::SPACE
                    && account.data.starts_with(PunishmentCase::DISCRIMINATOR)
            })
            .map(|(key, _)| key)
            .collect();
        assert_eq!(
            actual_cases, self.punishment_accounts,
            "punishment cases are unique and complete"
        );
    }
}

fn config_args(p: &Protocol, fee: u16) -> UpdateConfigArgs {
    let c = p.config();
    UpdateConfigArgs {
        withdrawal_cooldown_slots: c.withdrawal_cooldown_slots,
        minimum_node_deposit: c.minimum_node_deposit,
        reward_per_signature: c.reward_per_signature,
        max_settlement_delay_seconds: c.max_settlement_delay_seconds,
        challenger_bounty_bps: c.challenger_bounty_bps,
        x402_round_base: c.x402_round_base,
        min_signers: c.min_signers,
        epoch_len_seconds: c.epoch_len_seconds,
        ticket_grace_seconds: c.ticket_grace_seconds,
        target_tickets_per_epoch: c.target_tickets_per_epoch,
        min_availability_bps: c.min_availability_bps,
        min_availability_samples: c.min_availability_samples,
        protocol_fee_bps: fee,
    }
}

fn sequence(seed: u64, actions: &[u8]) -> [u32; 22] {
    let mut rng = InputRng(seed);
    let mut p = Protocol::builder()
        .with_env(TestEnv::from_seed(rng.seed()))
        .epoch_len_seconds(3_600)
        .ticket_grace_seconds(600)
        .min_availability_samples(3)
        .withdrawal_cooldown_slots(5)
        .max_settlement_delay_seconds(120)
        .start();
    let mut model = Ledger::default();
    for index in 0..4 {
        p.register_nodes(&format!("stateful:{seed}:{index}"), 1);
        model.registered(&p);
    }
    model.registry_version = p.current_version();
    model.epochs.insert(0, EpochModel::default());
    model.epochs.insert(1, EpochModel::default());
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let reporter = p.reporter();
    let authority = p.authority.insecure_clone();
    let reserve_destination = p.fund_usdc(&authority.pubkey(), 0);
    let mut historical = Vec::new();
    let mut executed = [0u32; 22];

    for (step, action) in actions.iter().enumerate() {
        let amount = 1 + rng.next_u64() % 1_000_000;
        let live: Vec<usize> = p
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| model.nodes[&n.pda].status == NodeStatus::Active)
            .map(|(i, _)| i)
            .collect();
        let open = p
            .live_epochs()
            .into_iter()
            .find(|e| p.env.epoch(*e).is_open(p.env.unix_timestamp()));
        let mut source = [19; 32];
        source[..8].copy_from_slice(&seed.to_le_bytes());
        source[8..16].copy_from_slice(&(step as u64).to_le_bytes());
        let action = usize::from(action % 22);
        let mut effective = false;
        match action {
            0 => {
                p.fund_treasury(amount);
                model.treasury += amount;
                effective = true;
            }
            1 => {
                if let Some(epoch) = open {
                    let committed = model.reserve
                        + model.nodes.values().map(|n| n.rewards).sum::<u64>()
                        + model.epochs.values().map(|e| e.pool - e.paid).sum::<u64>();
                    let gross = model.treasury - committed;
                    let ix = p.sweep_treasury_revenue_ix(epoch);
                    if gross == 0 {
                        let before =
                            AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
                        assert_molpha_error(
                            &p.env.send_err(&[ix], &[]),
                            MolphaError::NothingToSweep,
                        );
                        before.assert_unchanged(&p.env);
                    } else {
                        p.env.send_ok(&[ix], &[]);
                        let (fee, pool) = fee_split(gross, p.args.protocol_fee_bps);
                        model.reserve += fee;
                        model.epochs.get_mut(&epoch).unwrap().pool += pool;
                    }
                    effective = true;
                }
            }
            2 => {
                if let Some(epoch) = open.filter(|_| live.len() >= 3) {
                    let signed = p.sign_canonical(p.live_duty(3).source_id(source));
                    let registry = p.env.registry(signed.duty.registry_version);
                    let selected = p.selection_bits(&signed.duty);
                    p.submit_ticket(&submitter, &gateway, &signed);
                    let state = model.epochs.get_mut(&epoch).unwrap();
                    state.tickets += 1;
                    for bit in selected {
                        *state.selected.entry(registry.nodes[bit]).or_default() += 1;
                    }
                    for bit in crypto::bitmap_bits(&signed.signers_bitmap) {
                        *state.signed.entry(registry.nodes[bit]).or_default() += 1;
                    }
                    effective = true;
                }
            }
            3 | 4 => {
                if live.len() >= 3 {
                    let signed = p.sign_canonical(p.live_duty(3).source_id(source));
                    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
                    p.env.send_ok(std::slice::from_ref(&ix), &[&submitter]);
                    historical.push(signed);
                    if action % 22 == 3 {
                        let before =
                            AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
                        assert_molpha_error(
                            &p.env.send_err(&[ix], &[&submitter]),
                            MolphaError::FeedNotNewer,
                        );
                        before.assert_unchanged(&p.env);
                    }
                    effective = true;
                }
            }
            5 => {
                if let Some(epoch) = open.filter(|_| live.len() >= 3) {
                    let signed = p
                        .sign_canonical(p.live_duty(3).source_id(source))
                        .with_tampered_signature();
                    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
                    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
                    assert_molpha_error(
                        &p.env.send_err(&ixs, &[&submitter]),
                        MolphaError::InvalidAggregateSignature,
                    );
                    before.assert_unchanged(&p.env);
                    assert!(!model.epochs[&epoch].finalized);
                    effective = true;
                }
            }
            6 => {
                let epoch = model
                    .epochs
                    .iter()
                    .find(|(_, e)| !e.finalized)
                    .map(|(i, _)| *i)
                    .unwrap();
                let deadline = p.env.epoch(epoch).ticket_deadline();
                if p.env.unix_timestamp() < deadline {
                    p.env.warp_to(deadline);
                }
                p.env.send_ok(
                    &[p.finalize_epoch_ix(&authority.pubkey(), epoch)],
                    &[&authority],
                );
                let state = model.epochs.get_mut(&epoch).unwrap();
                for key in state.selected.keys() {
                    let weight = if state.below_floor(key) {
                        0
                    } else {
                        *state.signed.get(key).unwrap_or(&0)
                    };
                    if weight > 0 {
                        state.weights.insert(*key, weight);
                    }
                }
                state.finalized = true;
                model.epochs.insert(epoch + 2, EpochModel::default());
                effective = true;
            }
            7 => {
                let batch = model.epochs.iter().find_map(|(index, state)| {
                    let keys: Vec<_> = state
                        .weights
                        .keys()
                        .filter(|key| !state.credited.contains(key))
                        .take(2)
                        .copied()
                        .collect();
                    (!keys.is_empty()).then_some((*index, keys))
                });
                if let Some((epoch, keys)) = batch {
                    let slots: Vec<_> = keys
                        .iter()
                        .map(|key| p.roster_slot_of(epoch, key).unwrap())
                        .collect();
                    p.env
                        .send_ok(&[p.credit_epoch_rewards_ix(epoch, &slots)], &[]);
                    let state = model.epochs.get_mut(&epoch).unwrap();
                    let total: u64 = state.weights.values().sum();
                    for key in keys {
                        let node = model.nodes.get_mut(&key).unwrap();
                        let share = if node.status == NodeStatus::Tombstoned {
                            0
                        } else {
                            payout(state.pool, state.weights[&key], total)
                        };
                        node.rewards += share;
                        state.paid += share;
                        state.credited.insert(key);
                    }
                    effective = true;
                }
            }
            8 => {
                let index = (rng.next_u64() as usize) % p.nodes.len();
                let owner = p.nodes[index].owner.insecure_clone();
                let key = p.nodes[index].pda;
                let amount = model.nodes[&key].rewards;
                let ix = p.claim_node_rewards_ix(&owner.pubkey());
                if amount == 0 {
                    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
                    assert_molpha_error(
                        &p.env.send_err(&[ix], &[&owner]),
                        MolphaError::NothingToClaim,
                    );
                    before.assert_unchanged(&p.env);
                } else {
                    p.env.send_ok(&[ix], &[&owner]);
                    model.nodes.get_mut(&key).unwrap().rewards = 0;
                    model.treasury -= amount;
                }
                effective = true;
            }
            9 => {
                let amount = if model.reserve == 0 { 1 } else { model.reserve };
                let ix = p.withdraw_protocol_reserve_ix(
                    &authority.pubkey(),
                    &reserve_destination,
                    amount,
                );
                if model.reserve == 0 {
                    let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
                    assert_molpha_error(
                        &p.env.send_err(&[ix], &[&authority]),
                        MolphaError::InsufficientProtocolReserve,
                    );
                    before.assert_unchanged(&p.env);
                } else {
                    p.env.send_ok(&[ix], &[&authority]);
                    model.reserve -= amount;
                    model.treasury -= amount;
                }
                effective = true;
            }
            10 => {
                if live.len() > 3 {
                    let index = live[(rng.next_u64() as usize) % live.len()];
                    let owner = p.nodes[index].owner.insecure_clone();
                    p.env
                        .send_ok(&[p.deactivate_node_ix(index, &owner.pubkey())], &[&owner]);
                    model.nodes.get_mut(&p.nodes[index].pda).unwrap().status =
                        NodeStatus::Deactivated;
                    model.registry_version += 1;
                    effective = true;
                }
            }
            11 => {
                if live.len() < 8 {
                    p.register_nodes(&format!("stateful-register:{seed}:{step}"), 1);
                    model.registered(&p);
                    model.registry_version += 1;
                    effective = true;
                }
            }
            12 => {
                if let Some(index) = live.first() {
                    let node: Node = p.env.get(&p.nodes[*index].pda);
                    if node.key_rotated_at == 0
                        || p.env.unix_timestamp() - node.key_rotated_at >= 120
                    {
                        p.rotate_node_key(
                            *index,
                            crypto::node_keys(&format!("stateful-rotate:{seed}:{step}"), 1)
                                .remove(0),
                        );
                        model.registry_version += 1;
                        effective = true;
                    }
                }
            }
            13 => {
                let old = p.env.registry(p.current_version()).redundancy_buffer;
                let buffer = if old == 2 { 3 } else { 2 };
                p.env.send_ok(
                    &[p.update_registry_policy_ix(
                        &authority.pubkey(),
                        p.current_version() + 1,
                        buffer,
                    )],
                    &[&authority],
                );
                model.registry_version += 1;
                effective = true;
            }
            14 => {
                let fee = [0, 500, 10_000][(rng.next_u64() % 3) as usize];
                let args = config_args(&p, fee);
                p.env.send_ok(
                    &[molpha_tests::scenario::update_config_ix(
                        &authority.pubkey(),
                        &args,
                    )],
                    &[&authority],
                );
                p.args.protocol_fee_bps = fee;
                effective = true;
            }
            15 => {
                p.env.advance_seconds(61);
                effective = true;
            }
            16 => {
                let candidate = model.epochs.iter().find_map(|(epoch, state)| {
                    state
                        .selected
                        .keys()
                        .find(|key| {
                            state.finalized
                                && state.below_floor(key)
                                && model.nodes[*key].status == NodeStatus::Active
                                && model.nodes[*key].frozen_until <= p.env.epoch(*epoch).start
                                && !model.cases.contains(&(*epoch, **key))
                        })
                        .map(|key| (*epoch, *key))
                });
                if let Some((epoch, key)) = candidate {
                    let evidence = p.inactivity_evidence(epoch, &key);
                    let case = pda::punishment_case(&evidence.hash());
                    p.env.send_ok(
                        &[p.slash_inactivity_ix(&reporter.pubkey(), epoch, &key)],
                        &[&reporter],
                    );
                    let node = model.nodes.get_mut(&key).unwrap();
                    node.status = NodeStatus::Frozen;
                    node.frozen_until = p.env.unix_timestamp() + INACTIVITY_FREEZE_SECS;
                    model.cases.insert((epoch, key));
                    assert!(model.punishment_accounts.insert(case));
                    model.registry_version += 1;
                    effective = true;
                }
            }
            17 => {
                if let Some(index) = p
                    .nodes
                    .iter()
                    .position(|n| model.nodes[&n.pda].status == NodeStatus::Frozen)
                {
                    let owner = p.nodes[index].owner.pubkey();
                    p.env.send_ok(
                        &[p.reinstate_node_ix(&authority.pubkey(), &owner)],
                        &[&authority],
                    );
                    model.nodes.get_mut(&p.nodes[index].pda).unwrap().status = NodeStatus::Active;
                    model.registry_version += 1;
                    effective = true;
                }
            }
            18 => {
                if live.len() >= 3 {
                    let duty = p.live_duty(3).source_id(source);
                    let a = p.sign_canonical(duty.value([1; 32]));
                    let b = p.sign_canonical(duty.value([2; 32]));
                    let guilty = p.node_pdas_for_bitmap(p.current_version(), &a.signers_bitmap);
                    let evidence = EquivocationEvidence {
                        attestation_a: a.attestation(),
                        attestation_b: b.attestation(),
                        coalition_key_a: a.coalition_key,
                        coalition_key_b: b.coalition_key,
                    };
                    let case = pda::punishment_case(&evidence.hash());
                    p.env.send_ok(
                        &[p.slash_equivocation_ix(&reporter.pubkey(), evidence)],
                        &[&reporter],
                    );
                    let mut principal = 0;
                    let mut recovered = 0;
                    for key in guilty {
                        let node = model.nodes.get_mut(&key).unwrap();
                        principal += node.principal;
                        recovered += node.principal + node.rewards;
                        node.principal = 0;
                        node.rewards = 0;
                        node.status = NodeStatus::Tombstoned;
                    }
                    let (bounty, _) = fee_split(recovered, p.args.challenger_bounty_bps);
                    model.treasury += principal;
                    model.treasury -= bounty;
                    model.reserve += recovered - bounty;
                    assert!(model.punishment_accounts.insert(case));
                    model.registry_version += 1;
                    effective = true;
                }
            }
            19 => {
                if let Some(index) = p.nodes.iter().position(|n| {
                    let m = &model.nodes[&n.pda];
                    m.status == NodeStatus::Deactivated && m.principal > 0
                }) {
                    let key = p.nodes[index].pda;
                    let owner = p.nodes[index].owner.insecure_clone();
                    let node: Node = p.env.get(&key);
                    if p.env.slot() < node.withdrawable_slot {
                        p.env.advance_slots(node.withdrawable_slot - p.env.slot());
                    }
                    p.env
                        .send_ok(&[p.withdraw_node_deposit_ix(&owner.pubkey())], &[&owner]);
                    model.nodes.get_mut(&key).unwrap().principal = 0;
                    effective = true;
                }
            }
            20 => {
                if let Some(signed) = historical.last() {
                    let registry = p.env.registry(signed.duty.registry_version);
                    // Old rounds are only resubmitted while the snapshot and freshness allow it.
                    if registry.grace_active_until == 0
                        || p.env.unix_timestamp() < registry.grace_active_until
                    {
                        let mut args = signed.submit_args();
                        args.attestation.payload.timestamp =
                            p.env.unix_timestamp() as u64 * 1_000 + 1;
                        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
                        let before =
                            AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
                        assert_molpha_error(
                            &p.env.send_err(&[ix], &[&submitter]),
                            MolphaError::InvalidAggregateSignature,
                        );
                        before.assert_unchanged(&p.env);
                        effective = true;
                    }
                }
            }
            21 => {
                if let Some(open) = open {
                    let candidate = model
                        .epochs
                        .iter()
                        .find(|(epoch, state)| {
                            **epoch != open
                                && state.finalized
                                && state.credited.len() == state.weights.len()
                                && p.env.epoch(**epoch).closable(p.env.unix_timestamp())
                        })
                        .map(|(i, _)| *i);
                    if let Some(epoch) = candidate {
                        p.env.send_ok(&[p.close_epoch_ix(epoch, open)], &[]);
                        let state = model.epochs.remove(&epoch).unwrap();
                        model.epochs.get_mut(&open).unwrap().pool += state.pool - state.paid;
                        effective = true;
                    }
                }
            }
            _ => unreachable!(),
        }
        if effective {
            executed[action] += 1;
        }
        model.assert_matches(&p);
    }
    executed
}

#[test]
fn deterministic_trace_executes_every_modeled_action() {
    let mut actions = vec![0, 1, 3, 4, 20, 5];
    actions.extend(std::iter::repeat_n(2, 32));
    actions.extend([10, 11, 12, 13, 14, 15, 20, 9, 6]);
    actions.extend(std::iter::repeat_n(7, 32));
    actions.extend([8, 16, 17, 18, 19, 6, 21]);
    let executed = sequence(0x5452_4143_455f_5631, &actions);
    let missing: Vec<_> = executed
        .iter()
        .enumerate()
        .filter_map(|(action, count)| (*count == 0).then_some(action))
        .collect();
    assert!(
        missing.is_empty(),
        "modeled actions silently skipped: {missing:?}"
    );
}

#[test]
fn generated_protocol_sequences() {
    if let Ok(path) = std::env::var("MOLPHA_SECURITY_REPLAY") {
        let case: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let seed = case["seed"].as_str().unwrap().parse().unwrap();
        let actions: Vec<_> = case["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let first = sequence(seed, &actions);
        let replay = sequence(seed, &actions);
        assert_eq!(
            first, replay,
            "saved trace did not replay deterministically"
        );
        return;
    }
    let extended = std::env::var("MOLPHA_SECURITY_PROFILE").as_deref() == Ok("extended");
    let seed = std::env::var("MOLPHA_SECURITY_SEED")
        .map(|v| v.parse().expect("u64 security seed"))
        .unwrap_or(0x4d4f4c504841);
    let cases = if extended { 256 } else { 16 };
    let steps = if extended { 128 } else { 24 };
    let config = ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/security-review/stateful-regressions.txt"
        )))),
        ..ProptestConfig::default()
    };
    let strategy = (any::<u64>(), prop::collection::vec(0u8..22, 0..=steps - 6))
        .prop_map(|(seed, random)| (seed, [vec![0, 1, 2, 6, 7, 8], random].concat()));
    let mut runner = TestRunner::new(config);
    let result = runner.run(&strategy, |(seed, actions)| {
        if let Err(error) = catch_unwind(AssertUnwindSafe(|| {
            let first = sequence(seed, &actions);
            let replay = sequence(seed, &actions);
            assert_eq!(
                first, replay,
                "generated trace did not replay deterministically"
            );
        })) {
            let dir =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/security-review");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("stateful-replay.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "seed": seed.to_string(), "actions": actions,
                }))
                .unwrap(),
            )
            .unwrap();
            let message = error
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "sequence panic".into());
            return Err(TestCaseError::fail(message));
        }
        Ok(())
    });
    result.unwrap();
}
