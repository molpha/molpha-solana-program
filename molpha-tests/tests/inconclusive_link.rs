//! `register_inconclusive_id` and the `InconclusiveLink`: an INCONCLUSIVE attestation is signed
//! under `I(S) = keccak256("MOLPHA_INCONCLUSIVE_V1" || S)` by the committee selected for `S`, so
//! tickets and unsolicited-signing evidence must draw its committee under `S` (node-client
//! `docs/policy.md`).

use anchor_lang::error::ErrorCode;
use anchor_lang::prelude::Pubkey;
use molpha::error::MolphaError;
use molpha::events::InconclusiveIdRegistered;
use molpha::state::{
    inconclusive_id, Feed, InconclusiveLink, Node, NodeStatus, OffenceKind, PunishmentCase,
};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol, SignedAttestation};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;
const SIGS: u8 = 3;

/// Eight nodes, quorum 3 plus buffer 2: five selected per round, three sign.
fn setup() -> (Protocol, Keypair, Keypair) {
    let mut p = Protocol::start();
    p.register_nodes("inconclusive", NODE_COUNT);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    (p, gateway, submitter)
}

fn hex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 + 2 * i..4 + 2 * i], 16).unwrap();
    }
    out
}

/// `S` and the duties of `S` and `I(S)` at `base`'s round, for an `S` whose canonical signers are not
/// all in the committee `I(S)` itself would draw: the case the link exists for. `bits` is `S`'s
/// canonical set. Call before any link for `S` exists.
fn split_source(p: &Protocol, base: Duty) -> ([u8; 32], Duty, Duty, Vec<usize>) {
    for i in 1u8..=255 {
        let s = [i; 32];
        let ds = base.source_id(s);
        let di = base.source_id(inconclusive_id(&s));
        let canon: Vec<usize> = p.selection_bits(&ds)[..usize::from(SIGS)].to_vec();
        let under_i = p.selection_bits(&di);
        if !canon.iter().all(|b| under_i.contains(b)) {
            return (s, ds, di, canon);
        }
    }
    panic!("no source whose I(S) draws another committee");
}

fn register(p: &mut Protocol, source_id: [u8; 32]) {
    let payer = p.env.keypair();
    let ix = p.register_inconclusive_id_ix(&payer.pubkey(), source_id);
    p.env.send_ok(&[ix], &[&payer]);
}

#[test]
fn the_derivation_matches_the_node_vectors() {
    // new/source/testdata/policy-vectors.json in the node client.
    for (s, i) in [
        (
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xbafadaff7889fa857773e328110d584e3d6bccb300737429d995d12e41325f0e",
        ),
        (
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "0x4e9841f5548f5348f6ae8ca7c7afb5dd230dfc0bae566aba4d4441f2e1e8cd74",
        ),
        (
            "0xe371e37c37d592464b293ce89100c4c87ddcb667b901739c60827789b200b8af",
            "0xf0f8a58c104feffc9dc7dbb32452ec9829d7d63ee8bcc1a41ba2c5784a29a9ce",
        ),
    ] {
        assert_eq!(inconclusive_id(&hex32(s)), hex32(i), "I({s})");
    }
}

#[test]
fn a_link_records_s_at_the_address_of_i_s_once() {
    let (mut p, _gateway, _submitter) = setup();
    let s = [0x5a; 32];
    let payer = p.env.keypair();
    let ix = p.register_inconclusive_id_ix(&payer.pubkey(), s);
    let meta = p.env.send_ok(&[ix], &[&payer]);

    let key = pda::inconclusive_link(&inconclusive_id(&s));
    let link: InconclusiveLink = p.env.get(&key);
    assert_eq!(link.source_id, s);
    let event: InconclusiveIdRegistered = only_event(&meta);
    assert_eq!(event.source_id, s);
    assert_eq!(event.inconclusive_id, inconclusive_id(&s));
    assert_eq!(event.payer, payer.pubkey());

    // A link is written once; the address already holds it.
    p.env.advance_slots(1);
    let again = p.register_inconclusive_id_ix(&payer.pubkey(), s);
    p.env.send_err(&[again], &[&payer]);
    assert_eq!(p.env.get::<InconclusiveLink>(&key).source_id, s);

    let zero = p.register_inconclusive_id_ix(&payer.pubkey(), [0u8; 32]);
    let failed = p.env.send_err(&[zero], &[&payer]);
    assert_molpha_error(&failed, MolphaError::InvalidSourceId);
}

#[test]
fn an_inconclusive_ticket_counts_s_committee_once_the_link_exists() {
    let (mut p, gateway, submitter) = setup();
    let (s, ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    let signed = p.sign_duty_with_bits(di, &canon, None);

    // Without the link the program draws I(S)'s own committee, and S's signers are outside it.
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);

    register(&mut p, s);
    p.submit_ticket(&submitter, &gateway, &signed);

    let state = p.env.epoch(0);
    assert_eq!(state.tickets, 1);
    assert_eq!((state.sum_signed, state.sum_selected), (3, 5));
    let registry = p.env.registry(ds.registry_version);
    let selected: Vec<Pubkey> = p
        .selection_bits(&ds)
        .into_iter()
        .map(|b| registry.nodes[b])
        .collect();
    let roster: Vec<Pubkey> = state.roster[..usize::from(state.roster_len)]
        .iter()
        .map(|k| Pubkey::new_from_array(*k))
        .collect();
    assert_eq!(roster, selected, "the roster is S's selection");
}

#[test]
fn without_a_link_an_honest_inconclusive_committee_reads_as_unsolicited() {
    // The hazard the link removes: evidence is filed against S's committee for signing I(S).
    let (mut p, _gateway, _submitter) = setup();
    let (_s, _ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    let signed = p.sign_duty_with_bits(di, &canon, None);
    let reporter = p.reporter();
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), unsolicited(&signed));
    p.env.send_ok(&[ix], &[&reporter]);
}

#[test]
fn with_the_link_an_honest_inconclusive_committee_is_not_unsolicited() {
    let (mut p, _gateway, _submitter) = setup();
    let (s, _ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    register(&mut p, s);
    let signed = p.sign_duty_with_bits(di, &canon, None);
    let reporter = p.reporter();
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), unsolicited(&signed));
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersFullySelected);
}

#[test]
fn a_real_intruder_in_an_inconclusive_attestation_is_still_slashed() {
    let (mut p, _gateway, _submitter) = setup();
    let (s, ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    register(&mut p, s);
    let selected = p.selection_bits(&ds);
    let intruder = (0..NODE_COUNT)
        .find(|b| !selected.contains(b))
        .expect("a buffer of 2 over 8 nodes leaves someone out");
    let mut bits = canon[..2].to_vec();
    bits.push(intruder);
    bits.sort_unstable();
    let signed = p.sign_duty_with_bits(di, &bits, None);

    let reporter = p.reporter();
    let evidence = unsolicited(&signed);
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), evidence.clone());
    p.env.send_ok(&[ix], &[&reporter]);

    let node_key = p.env.registry(ds.registry_version).nodes[intruder];
    let node: Node = p.env.get(&node_key);
    assert_eq!(node.status, NodeStatus::Tombstoned);
    for bit in &bits[..] {
        if *bit != intruder {
            let honest: Node = p.env.get(&p.env.registry(ds.registry_version).nodes[*bit]);
            assert_eq!(
                honest.status,
                NodeStatus::Active,
                "bit {bit} was selected for S"
            );
        }
    }
    let case: PunishmentCase = p.env.get(&pda::punishment_case(
        &molpha::state::Offence::UnsolicitedSigning(evidence).hash(),
    ));
    assert_eq!(case.offence, OffenceKind::UnsolicitedSigning);
}

#[test]
fn the_link_account_cannot_be_swapped() {
    let (mut p, gateway, submitter) = setup();
    let (s, _ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    register(&mut p, s);
    let signed = p.sign_duty_with_bits(di, &canon, None);

    // A ticket naming another address for the link is refused by its seeds.
    let mut ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let link = pda::inconclusive_link(&di.source_id);
    let other = pda::inconclusive_link(&[0x77; 32]);
    let ticket = ixs.last_mut().unwrap();
    let meta = ticket
        .accounts
        .iter_mut()
        .find(|m| m.pubkey == link)
        .expect("the ticket names the link");
    meta.pubkey = other;
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_anchor_error(&failed, ErrorCode::ConstraintSeeds);

    // Unsolicited-signing evidence names it as its last account: another address, or none, is
    // refused, so an accuser cannot have S's committee drawn under I(S).
    let reporter = p.reporter();
    let mut swapped = p.slash_unsolicited_signing_ix(&reporter.pubkey(), unsolicited(&signed));
    assert_eq!(swapped.accounts.last().unwrap().pubkey, link);
    swapped.accounts.last_mut().unwrap().pubkey = other;
    let failed = p.env.send_err(&[swapped], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidInconclusiveLink);
    let mut omitted = p.slash_unsolicited_signing_ix(&reporter.pubkey(), unsolicited(&signed));
    omitted.accounts.pop();
    let failed = p.env.send_err(&[omitted], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidInconclusiveLink);
}

#[test]
fn an_inconclusive_attestation_lands_on_its_own_feed() {
    let (mut p, _gateway, submitter) = setup();
    let (s, ds, di, canon) = split_source(&p, p.live_duty(SIGS));
    register(&mut p, s);
    let signed = p.sign_duty_with_bits(di, &canon, None);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    p.env.send_ok(&[ix], &[&submitter]);

    let feed: Feed = p
        .env
        .get(&pda::feed(&di.source_id, SIGS, &submitter.pubkey()));
    assert_eq!(feed.source_id, di.source_id);
    assert!(!p
        .env
        .account_exists(&pda::feed(&ds.source_id, SIGS, &submitter.pubkey())));
}

fn unsolicited(signed: &SignedAttestation) -> molpha::state::UnsolicitedSigningEvidence {
    molpha::state::UnsolicitedSigningEvidence {
        attestation: signed.attestation(),
        coalition_key: signed.coalition_key,
    }
}

#[test]
fn a_link_for_an_id_that_does_not_derive_is_refused() {
    let (mut p, _gateway, _submitter) = setup();
    let payer = p.env.keypair();
    // Claiming that [0x22; 32] is I([0x11; 32]) would let S's committee shield another source.
    let ix = p.register_inconclusive_id_raw_ix(&payer.pubkey(), [0x11; 32], [0x22; 32]);
    let failed = p.env.send_err(&[ix], &[&payer]);
    assert_molpha_error(&failed, MolphaError::InvalidInconclusiveLink);
    assert!(!p.env.account_exists(&pda::inconclusive_link(&[0x22; 32])));
}
