//! The JS corpus goes through real registration and the deployed instruction decoder.

use anchor_lang::AnchorDeserialize;
use molpha::error::MolphaError;
use molpha::instructions::SubmitAttestationArgs;
use molpha::state::Feed;
use molpha_tests::crypto::NodeKey;
use molpha_tests::env::{assert_molpha_error, TestEnv};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use molpha_tests::security::AccountSnapshot;
use solana_signer::Signer;

fn unhex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn javascript_wire_signatures_execute_through_sbf_without_reencoding_loss() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/security-boundary.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut p = Protocol::builder()
            .with_env(TestEnv::from_seed([82; 32]))
            .start();
        for value in 1..=4 {
            let mut scalar = [0; 32];
            scalar[31] = value;
            p.register_node_with_key(NodeKey::from_secret(scalar));
        }
        let mut scalar = [0; 32];
        scalar[31] = 5;
        p.register_provider(NodeKey::from_secret(scalar));
        assert_eq!(p.current_version(), 5);
        let wire = unhex(case["args_hex"].as_str().unwrap());
        let args = SubmitAttestationArgs::try_from_slice(&wire).unwrap();
        assert_eq!(borsh::to_vec(&args).unwrap(), wire);
        let payload = &args.attestation.payload;
        assert_eq!(
            payload.timestamp.to_string(),
            case["timestamp"].as_str().unwrap()
        );
        if case["outcome"] == "ok" {
            assert_eq!(
                args.attestation.message_hash().to_vec(),
                unhex(case["message_hash"].as_str().unwrap())
            );
        }
        p.env.warp_to((payload.timestamp / 1_000) as i64);
        let source = payload.source_id;
        let expected_value = payload.value;
        let submitter = p.env.keypair();
        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
        // The bytes emitted by the JS codec must be exactly what the Rust instruction sends.
        assert_eq!(&ix.data[8..], wire.as_slice());
        let before = AccountSnapshot::capture(&p.env, std::slice::from_ref(&ix), &[]);
        match case["outcome"].as_str().unwrap() {
            "ok" => {
                p.env.send_ok(&[ix], &[&submitter]);
                let feed: Feed = p.env.get(&pda::feed(&source, 3, &submitter.pubkey()));
                assert_eq!(feed.value, expected_value, "{}", case["name"]);
            }
            "InvalidAggregateSignature" => {
                let failed = p.env.send_err(&[ix], &[&submitter]);
                assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);
                before.assert_unchanged(&p.env);
            }
            "InvalidCoalitionKey" => {
                let failed = p.env.send_err(&[ix], &[&submitter]);
                assert_molpha_error(&failed, MolphaError::InvalidCoalitionKey);
                before.assert_unchanged(&p.env);
            }
            other => panic!("unknown fixture outcome {other}"),
        }
        p.assert_solvent();
    }
}
