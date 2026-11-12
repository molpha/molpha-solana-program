//! Valid native signatures with adversarial introspection layouts, distinct from bad signatures.

use anchor_lang::solana_program::instruction::Instruction;
use molpha::error::MolphaError;
use molpha::state::dispatch_message;
use molpha_tests::ed25519::{sign_ed25519, ED25519_PROGRAM_ID};
use molpha_tests::env::{assert_molpha_error, TestEnv};
use molpha_tests::scenario::Protocol;
use molpha_tests::security::AccountSnapshot;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn multiple(entries: &[(&Keypair, Vec<u8>)], padding: usize) -> Instruction {
    let mut data = vec![0; 2 + 14 * entries.len() + padding];
    data[0] = entries.len() as u8;
    for (index, (signer, message)) in entries.iter().enumerate() {
        let key_offset = data.len();
        data.extend_from_slice(signer.pubkey().as_ref());
        let message_offset = data.len();
        data.extend_from_slice(message);
        let signature_offset = data.len();
        data.extend_from_slice(signer.sign_message(message).as_ref());
        let fields = [
            signature_offset as u16,
            u16::MAX,
            key_offset as u16,
            u16::MAX,
            message_offset as u16,
            message.len() as u16,
            u16::MAX,
        ];
        for (field, value) in fields.into_iter().enumerate() {
            let start = 2 + index * 14 + field * 2;
            data[start..start + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
    Instruction {
        program_id: ED25519_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}

fn setup() -> (Protocol, Keypair, Keypair) {
    let mut p = Protocol::builder()
        .with_env(TestEnv::from_seed([81; 32]))
        .start();
    p.register_nodes("dispatch-layouts", 4);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    (p, gateway, submitter)
}

#[test]
fn a_matching_later_signature_and_relocated_regions_are_accepted() {
    let (mut p, gateway, submitter) = setup();
    let unrelated = p.env.keypair();
    let signed = p.sign_canonical(p.live_duty(3));
    let message = dispatch_message(&molpha::ID, &signed.duty.payload());
    let dispatch = multiple(
        &[(&unrelated, vec![1; 9]), (&gateway, message.to_vec())],
        17,
    );
    let ticket = p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed);
    p.env.send_ok(&[dispatch, ticket], &[&submitter]);
    assert_eq!(p.env.epoch(0).tickets, 1);
    p.assert_solvent();
}

#[test]
fn a_verified_wrong_message_among_multiple_signatures_does_not_authorize_a_ticket() {
    let (mut p, gateway, submitter) = setup();
    let unrelated = p.env.keypair();
    let signed = p.sign_canonical(p.live_duty(3));
    let message = dispatch_message(&molpha::ID, &signed.duty.payload());
    let dispatch = multiple(
        &[(&gateway, vec![0; 32]), (&unrelated, message.to_vec())],
        3,
    );
    let ixs = [
        dispatch,
        p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed),
    ];
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);
    before.assert_unchanged(&p.env);
}

#[test]
fn explicit_key_or_message_instruction_references_are_rejected_after_native_verification() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let message = dispatch_message(&molpha::ID, &signed.duty.payload());
    // Compute budget occupies index 0; the Ed25519 instruction is index 1. These references
    // resolve to its own valid bytes in the native verifier, but violate Molpha's wire policy.
    for field in [8usize, 14] {
        let mut dispatch = sign_ed25519(&gateway, &message);
        dispatch.data[field..field + 2].copy_from_slice(&1u16.to_le_bytes());
        let ixs = [
            dispatch,
            p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed),
        ];
        let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
        let failed = p.env.send_err(&ixs, &[&submitter]);
        assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);
        before.assert_unchanged(&p.env);
    }
}

#[test]
fn referencing_a_previous_verified_dispatch_is_not_self_contained() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let message = dispatch_message(&molpha::ID, &signed.duty.payload());
    let first = sign_ed25519(&gateway, &message);
    let mut second = first.clone();
    for field in [4usize, 8, 14] {
        second.data[field..field + 2].copy_from_slice(&1u16.to_le_bytes());
    }
    let ixs = [
        first,
        second,
        p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed),
    ];
    let before = AccountSnapshot::capture(&p.env, &ixs, &[]);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::DispatchSigInvalid);
    before.assert_unchanged(&p.env);
}

#[test]
fn signature_bytes_may_reference_the_same_verified_instruction_explicitly() {
    let (mut p, gateway, submitter) = setup();
    let signed = p.sign_canonical(p.live_duty(3));
    let message = dispatch_message(&molpha::ID, &signed.duty.payload());
    let mut dispatch = sign_ed25519(&gateway, &message);
    dispatch.data[4..6].copy_from_slice(&1u16.to_le_bytes());
    let ticket = p.submit_ticket_ix(&submitter.pubkey(), &gateway.pubkey(), 0, &signed);
    p.env.send_ok(&[dispatch, ticket], &[&submitter]);
    assert_eq!(p.env.epoch(0).tickets, 1);
}
