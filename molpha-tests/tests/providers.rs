//! Provider signers: registration through `register_node`, ejection, key rotation, and what a
//! provider bit does and does not do. A provider co-signs the aggregate at bit `64 + slot`; its bit proves it signed and
//! counts toward the quorum like a node's (at most one per attestation, so a provider round is
//! `required - 1` nodes plus the provider). It is never part of the selection, a credit or a
//! punishment.
#![allow(clippy::result_large_err)] // see the note in `molpha_tests`

use molpha::error::MolphaError;
use molpha::events::{
    ProviderEjected, ProviderKeyRotated, ProviderP2pKeyUpdated, ProviderRegistered,
    RegistryUpdated, RegistryVersionReason,
};
use molpha::instructions::MemberEndpoint;
use molpha::state::{
    Feed, KeyXClaim, NodeStatus, Provider, ProviderStatus, RegistryState, NODE_SLOTS,
    PROVIDER_BIT_BASE,
};
use molpha_tests::crypto::{self, NodeKey};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::{Duty, Protocol};
use solana_keypair::Keypair;
use solana_signer::Signer;

const NODE_COUNT: usize = 8;

fn fresh_key(domain: &str) -> NodeKey {
    crypto::node_keys(domain, 1).remove(0)
}

fn protocol() -> Protocol {
    let mut p = Protocol::start();
    p.register_nodes("providers", NODE_COUNT);
    p
}

/// The key with the same x coordinate and the other parity: `-P`, whose secret is `n - k`.
fn negated(key: &NodeKey) -> NodeKey {
    NodeKey::from_secret(crypto::negate_mod_n(&key.secret))
}

/// A duty at the clock now, against the current version, with the default source.
fn live(p: &Protocol, required: u8) -> Duty {
    p.live_duty(required)
}

// ---------------------------------------------------------------------------
// registration: `register_node` with the provider account
// ---------------------------------------------------------------------------

/// Send a provider registration signed by the protocol authority and the provider authority.
fn send_registration(
    p: &mut Protocol,
    authority: &Keypair,
    ix: anchor_lang::solana_program::instruction::Instruction,
) -> litesvm::types::TransactionResult {
    let protocol_authority = p.authority.insecure_clone();
    p.env.send(&[ix], &[&protocol_authority, authority])
}

#[test]
fn registering_puts_the_provider_in_the_registry_and_reserves_its_key() {
    let mut p = protocol();
    let key = fresh_key("register");
    let authority = p.env.keypair();
    let p2p = p.p2p_key();
    let args = p.register_provider_args(&authority.pubkey(), &key, p2p);
    assert_eq!(p.next_provider_id(), 1);
    let version = p.current_version();
    let deposits = p.node_deposit_balance();
    let ix = p.register_provider_ix(&authority.pubkey(), &args);
    let meta = send_registration(&mut p, &authority, ix).unwrap();

    let provider: Provider = p.env.get(&pda::provider(1));
    assert_eq!(provider.id, 1);
    assert_eq!(provider.authority, authority.pubkey());
    assert_eq!((provider.x, provider.y), (key.x, key.y));
    assert_eq!(provider.p2p_key, p2p);
    // Provider slot 0 is reserved for the Molpha signer, so the first provider is slot 1.
    assert_eq!(provider.slot, 1);
    assert_eq!(provider.status, ProviderStatus::Active);
    assert_eq!(provider.key_rotated_at, 0);

    // In the version this published, at provider slot 1 (bit 65) right after the node table, with
    // the nodes untouched and the reserved slot 0 a hole: two records more, not 58.
    assert_eq!(p.current_version(), version + 1);
    let registry = p.env.registry(version + 1);
    assert_eq!(
        (registry.node_count as usize, registry.provider_count),
        (NODE_COUNT, 2)
    );
    assert_eq!(
        registry.data_len,
        molpha::state::Registry::space(NODE_COUNT + 2)
    );
    assert_eq!(registry.nodes[PROVIDER_BIT_BASE + 1], pda::provider(1));
    assert_eq!(registry.keys[PROVIDER_BIT_BASE + 1], (key.x, key.y));
    assert!(!registry.is_live(PROVIDER_BIT_BASE));
    assert_eq!(registry.live_slots().len(), NODE_COUNT + 1);

    // The key's x is reserved for the provider, and no deposit was taken.
    let claim: KeyXClaim = p.env.get(&pda::key_x_claim(&key.x));
    assert_eq!(claim.owner, pda::provider(1));
    assert_eq!(
        p.env
            .get::<RegistryState>(&pda::registry_state())
            .next_provider_id,
        2
    );
    assert_eq!(p.node_deposit_balance(), deposits);

    let event: ProviderRegistered = only_event(&meta);
    assert_eq!(
        (event.id, event.slot, event.registry_version),
        (1, 1, version + 1)
    );
    assert_eq!(event.authority, authority.pubkey());
    assert_eq!(event.p2p_key, p2p);
}

#[test]
fn registering_a_provider_needs_the_protocol_authority() {
    let mut p = protocol();
    let key = fresh_key("register-auth");
    let authority = p.env.keypair();
    let p2p = p.p2p_key();
    let args = p.register_provider_args(&authority.pubkey(), &key, p2p);
    let mut ix = p.register_provider_ix(&authority.pubkey(), &args);
    let impostor = p.env.keypair();
    ix.accounts[0].pubkey = impostor.pubkey();
    let failed = p.env.send_err(&[ix], &[&impostor, &authority]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
    assert!(!p.env.account_exists(&pda::provider(1)));
}

#[test]
fn provider_ids_are_assigned_in_order_and_never_reused() {
    let mut p = protocol();
    let a = p.register_provider(fresh_key("id-a"));
    let b = p.register_provider(fresh_key("id-b"));
    assert_eq!((p.providers[a].id, p.providers[b].id), (1, 2));

    // Ejecting frees no id: the next registration gets 3.
    p.eject_provider(a);
    let c = p.register_provider(fresh_key("id-c"));
    assert_eq!(p.providers[c].id, 3);
    assert_eq!(p.provider_account(1).status, ProviderStatus::Removed);
}

/// A provider proves possession with the node construction: `MOLPHA_NODE_POP_V1` over the node id
/// of the registering authority, its key and its salt.
#[test]
fn the_proof_of_possession_is_the_node_one_bound_to_the_registering_authority() {
    let mut p = protocol();
    let key = fresh_key("pop");
    let other_key = fresh_key("pop-other");
    let authority = p.env.keypair();
    let p2p = p.p2p_key();
    let args = p.register_provider_args(&authority.pubkey(), &key, p2p);

    // A proof made by someone who does not hold the key (the rogue-key case).
    let node_id = crypto::derive_node_id(
        &authority.pubkey(),
        &key.compressed,
        &args.registration_salt,
    );
    let mut forged = args.clone();
    (forged.pop_sig_r, forged.pop_sig_s) = crypto::sign_pop_with(
        &molpha::ID,
        &node_id,
        &key.compressed,
        &other_key.secret,
        &[7; 32],
    );
    let ix = p.register_provider_ix(&authority.pubkey(), &forged);
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    // A genuine proof, replayed by another authority: the node id names the authority.
    let thief = p.env.keypair();
    let ix = p.register_provider_ix(&thief.pubkey(), &args);
    let failed = send_registration(&mut p, &thief, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    // The same proof under another salt.
    let mut resalted = args.clone();
    resalted.registration_salt[31] ^= 1;
    let ix = p.register_provider_ix(&authority.pubkey(), &resalted);
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    // A zero salt is refused before the proof is checked, as for a node.
    let mut zero_salt = args.clone();
    zero_salt.registration_salt = [0; 32];
    let ix = p.register_provider_ix(&authority.pubkey(), &zero_salt);
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::InvalidRegistrationSalt);

    assert_eq!(p.next_provider_id(), 1);
    assert!(!p.env.account_exists(&pda::provider(1)));
    assert!(!p.env.account_exists(&pda::key_x_claim(&key.x)));
}

#[test]
fn a_key_that_is_not_a_curve_point_is_refused() {
    let mut p = protocol();
    let key = fresh_key("off-curve");
    let authority = p.env.keypair();
    let p2p = p.p2p_key();
    let mut args = p.register_provider_args(&authority.pubkey(), &key, p2p);
    args.secp256k1_pubkey_compressed[0] = 0x05;
    let ix = p.register_provider_ix(&authority.pubkey(), &args);
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::PubkeyNotOnCurve);
}

#[test]
fn the_p2p_key_must_be_an_ed25519_point() {
    let mut p = protocol();
    let key = fresh_key("p2p");
    let authority = p.env.keypair();
    // All zeros is not a key, and neither is a 32-byte string that does not decompress.
    let not_a_point = (0u8..=255)
        .map(|y| {
            let mut bytes = [0u8; 32];
            bytes[0] = y;
            bytes
        })
        .find(|bytes| !anchor_lang::prelude::Pubkey::new_from_array(*bytes).is_on_curve())
        .expect("some small y is off the curve");
    for bad in [[0u8; 32], not_a_point] {
        let args = p.register_provider_args(&authority.pubkey(), &key, bad);
        let ix = p.register_provider_ix(&authority.pubkey(), &args);
        let failed = send_registration(&mut p, &authority, ix).unwrap_err();
        assert_molpha_error(&failed, MolphaError::InvalidP2pKey);
    }
    assert_eq!(p.next_provider_id(), 1);
}

/// The endpoint's variant names the kind being registered; it must match the one account passed.
#[test]
fn the_endpoint_kind_must_match_the_account_registered() {
    let mut p = protocol();
    let key = fresh_key("kind");
    let authority = p.env.keypair();
    p.fund_usdc(&authority.pubkey(), p.args.minimum_node_deposit * 4);
    let p2p = p.p2p_key();
    let version = p.current_version();

    // A node endpoint with the provider account.
    let mut args = p.register_provider_args(&authority.pubkey(), &key, p2p);
    args.endpoint = MemberEndpoint::Node {
        ip: [10, 0, 0, 1],
        port: 9_000,
    };
    let ix = p.register_provider_ix(&authority.pubkey(), &args);
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::MemberKindMismatch);

    // A provider endpoint with the node account.
    let mut args = p.register_node_args(&authority.pubkey(), &key, [5; 32], 9_000);
    args.endpoint = MemberEndpoint::Provider { p2p_key: p2p };
    let ix = p.register_node_ix(&authority.pubkey(), &args, &p.authority.pubkey());
    let failed = send_registration(&mut p, &authority, ix).unwrap_err();
    assert_molpha_error(&failed, MolphaError::MemberKindMismatch);

    // Both accounts, or neither.
    let args = p.register_provider_args(&authority.pubkey(), &key, p2p);
    let provider_ix = p.register_provider_ix(&authority.pubkey(), &args);
    let node_ix = p.register_node_ix(&authority.pubkey(), &args, &p.authority.pubkey());
    // `node` and `provider` in `RegisterNode`'s account order.
    let (node_slot, provider_slot) = (6, 7);
    assert_eq!(
        node_ix.accounts[node_slot].pubkey,
        pda::node(&authority.pubkey())
    );
    assert_eq!(provider_ix.accounts[provider_slot].pubkey, pda::provider(1));
    let mut both = node_ix.clone();
    both.accounts[provider_slot] = provider_ix.accounts[provider_slot].clone();
    let failed = send_registration(&mut p, &authority, both).unwrap_err();
    assert_molpha_error(&failed, MolphaError::ExactlyOneMember);
    let mut neither = provider_ix.clone();
    neither.accounts[provider_slot] = node_ix.accounts[provider_slot].clone();
    assert_eq!(neither.accounts[node_slot].pubkey, molpha::ID);
    let failed = send_registration(&mut p, &authority, neither).unwrap_err();
    assert_molpha_error(&failed, MolphaError::ExactlyOneMember);

    assert_eq!(p.current_version(), version);
    assert!(!p.env.account_exists(&pda::key_x_claim(&key.x)));
}

#[test]
fn a_duplicate_or_negated_key_cannot_be_registered_by_a_node_or_a_provider() {
    let mut p = protocol();
    let key = fresh_key("x-only");
    p.register_provider(key.clone());
    let authority = p.env.keypair();
    let p2p = p.p2p_key();

    // The same key again, and its negation: same x, other parity. A claim on the compressed
    // bytes would miss the negation.
    let negation = negated(&key);
    assert_eq!(negation.x, key.x);
    assert_ne!(negation.compressed, key.compressed);
    for taken in [key.clone(), negation] {
        let args = p.register_provider_args(&authority.pubkey(), &taken, p2p);
        let ix = p.register_provider_ix(&authority.pubkey(), &args);
        let failed = send_registration(&mut p, &authority, ix).unwrap_err();
        assert!(
            failed
                .meta
                .logs
                .iter()
                .any(|line| line.contains("already in use")),
            "{:?}",
            failed.meta.logs
        );
    }
    assert_eq!(p.next_provider_id(), 2);

    // A node registering the provider's key, or its negation, is refused for the same reason.
    for node_key in [key.clone(), negated(&key)] {
        let owner = p.env.keypair();
        p.fund_usdc(&owner.pubkey(), p.args.minimum_node_deposit * 4);
        let args = p.register_node_args(&owner.pubkey(), &node_key, [3; 32], 9_100);
        let ix = p.register_node_ix(&owner.pubkey(), &args, &p.authority.pubkey());
        let failed = send_registration(&mut p, &owner, ix).unwrap_err();
        assert!(failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")));
    }

    // And a provider registering a node's key, or its negation.
    let node_key = p.nodes[0].key.clone();
    for provider_key in [node_key.clone(), negated(&node_key)] {
        let args = p.register_provider_args(&authority.pubkey(), &provider_key, p2p);
        let ix = p.register_provider_ix(&authority.pubkey(), &args);
        let failed = send_registration(&mut p, &authority, ix).unwrap_err();
        assert!(failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")));
    }
    assert_eq!(p.next_provider_id(), 2);
}

#[test]
fn a_rotated_node_cannot_rotate_to_a_providers_key() {
    let mut p = protocol();
    let key = fresh_key("rotate-to-provider-key");
    p.register_provider(key.clone());
    let owner = p.nodes[0].owner.insecure_clone();
    let proof = p.key_rotation_proof(0, &key);
    let ix = p.rotate_node_key_ix(0, proof);
    let failed = p.env.send_err(&[ix], &[&owner]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|l| l.contains("already in use")));
}

#[test]
fn providers_take_the_lowest_free_slot_and_an_ejected_slot_is_reused_at_once() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("slot-a"));
    let b = p.add_active_provider(fresh_key("slot-b"));
    let c = p.add_active_provider(fresh_key("slot-c"));
    assert_eq!(
        [
            p.providers[a].slot,
            p.providers[b].slot,
            p.providers[c].slot
        ],
        [Some(1), Some(2), Some(3)]
    );

    // Slot 2 frees; the next provider takes it, and binds by id so it inherits no trust.
    p.eject_provider(b);
    let d = p.add_active_provider(fresh_key("slot-d"));
    assert_eq!(p.providers[d].slot, Some(2));
    assert_ne!(p.providers[d].id, p.providers[b].id);
    let registry = p.env.registry(p.current_version());
    assert_eq!(registry.nodes[PROVIDER_BIT_BASE + 2], p.providers[d].pda);
    assert_eq!(registry.nodes[PROVIDER_BIT_BASE + 1], p.providers[a].pda);
    assert_eq!(registry.nodes[PROVIDER_BIT_BASE + 3], p.providers[c].pda);
}

#[test]
fn registering_a_provider_does_not_move_or_change_any_node() {
    let mut p = protocol();
    let before = p.env.registry(p.current_version());
    p.add_active_provider(fresh_key("nodes-unmoved"));
    let after = p.env.registry(p.current_version());
    for node in 0..NODE_COUNT {
        assert_eq!(after.nodes[node], before.nodes[node]);
        assert_eq!(after.keys[node], before.keys[node]);
    }
    assert_eq!(after.redundancy_buffer, before.redundancy_buffer);
    // The previous version is stamped with the time its grace ends, as for a node change.
    assert_ne!(p.env.registry(before.version).grace_active_until, 0);
}

#[test]
fn a_node_registered_after_a_provider_still_takes_a_node_slot() {
    let mut p = protocol();
    let provider = p.add_active_provider(fresh_key("node-after"));
    p.register_node_with_key(fresh_key("node-after-node"));
    let registry = p.env.registry(p.current_version());
    assert_eq!(usize::from(p.nodes[NODE_COUNT].index), NODE_COUNT);
    assert_eq!(registry.nodes[NODE_COUNT], p.nodes[NODE_COUNT].pda);
    assert_eq!(
        registry.nodes[p.providers[provider].bit()],
        p.providers[provider].pda
    );
    // The node table grew by one and the provider table moved after it: nothing changed a bit.
    assert_eq!(
        (registry.node_count as usize, registry.provider_count),
        (NODE_COUNT + 1, 2)
    );
    assert_eq!(registry.records(), NODE_COUNT + 3);
}

// ---------------------------------------------------------------------------
// eject_provider and the authority
// ---------------------------------------------------------------------------

#[test]
fn only_the_provider_authority_can_eject_it() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("eject-auth"));
    let id = p.providers[index].id;
    let version = p.current_version();

    // Not a stranger, not the protocol authority that admitted it.
    let stranger = p.env.keypair();
    let protocol_authority = p.authority.insecure_clone();
    for who in [&stranger, &protocol_authority] {
        let ix = p.eject_provider_ix(&who.pubkey(), id);
        let failed = p.env.send_err(&[ix], &[who]);
        assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);
    }

    assert_eq!(p.provider_account(id).status, ProviderStatus::Active);
    assert_eq!(p.current_version(), version);
}

#[test]
fn ejecting_publishes_a_version_without_the_provider_and_nothing_else() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("eject-a"));
    let b = p.add_active_provider(fresh_key("eject-b"));
    let version = p.current_version();
    let authority = p.providers[a].authority.insecure_clone();
    let ix = p.eject_provider_ix(&authority.pubkey(), p.providers[a].id);
    let meta = p.env.send_ok(&[ix], &[&authority]);

    assert_eq!(p.current_version(), version + 1);
    assert_eq!(
        p.provider_account(p.providers[a].id).status,
        ProviderStatus::Removed
    );
    let registry = p.env.registry(version + 1);
    // Slot 1 is a hole inside the table; slot 2 keeps its record.
    assert!(!registry.is_live(PROVIDER_BIT_BASE + 1));
    assert_eq!(registry.nodes[PROVIDER_BIT_BASE + 2], p.providers[b].pda);
    assert_eq!(registry.node_count as usize, NODE_COUNT);
    assert_eq!(registry.provider_count, 3);
    let event: ProviderEjected = only_event(&meta);
    assert_eq!((event.id, event.slot), (p.providers[a].id, 1));
    let updated: RegistryUpdated = only_event(&meta);
    assert_eq!(
        updated.reason as u8,
        RegistryVersionReason::ProviderEjected as u8
    );
    assert_eq!(
        (usize::from(updated.node_count), updated.provider_count),
        (NODE_COUNT, 3)
    );

    // Ejecting the last record trims the table back.
    let authority = p.providers[b].authority.insecure_clone();
    let ix = p.eject_provider_ix(&authority.pubkey(), p.providers[b].id);
    p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(
        p.env.registry(p.current_version()).node_count as usize,
        NODE_COUNT
    );

    // Terminal.
    let ix = p.eject_provider_ix(&authority.pubkey(), p.providers[b].id);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::ProviderRemoved);
}

/// The protocol authority admits nodes and providers, but has no lane that removes either.
#[test]
fn the_protocol_authority_removes_no_one() {
    let mut p = protocol();
    let provider = p.add_active_provider(fresh_key("authority-removes-no-one"));
    let version = p.current_version();
    let authority = p.authority.insecure_clone();

    let ix = p.deactivate_node_ix(0, &authority.pubkey());
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);

    let ix = p.eject_provider_ix(&authority.pubkey(), p.providers[provider].id);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);

    assert_eq!(p.current_version(), version);
    assert_eq!(
        p.env.get::<molpha::state::Node>(&p.nodes[0].pda).status,
        NodeStatus::Active
    );
    assert_eq!(
        p.provider_account(p.providers[provider].id).status,
        ProviderStatus::Active
    );
}

// ---------------------------------------------------------------------------
// `update_node_endpoint` with the provider account: the p2p key
// ---------------------------------------------------------------------------

#[test]
fn the_authority_rotates_the_p2p_key_without_a_registry_version() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("p2p-rotate"));
    let id = p.providers[index].id;
    let version = p.current_version();
    let authority = p.providers[index].authority.insecure_clone();
    let old = p.providers[index].p2p_key;
    let new = p.p2p_key();

    let ix = p.update_provider_p2p_key_ix(&authority.pubkey(), id, new);
    let meta = p.env.send_ok(&[ix], &[&authority]);
    assert_eq!(p.provider_account(id).p2p_key, new);
    assert_eq!(p.current_version(), version);
    let event: ProviderP2pKeyUpdated = only_event(&meta);
    assert_eq!((event.old_p2p_key, event.new_p2p_key), (old, new));

    // Not for anyone else, not to a non-key, and not once removed.
    let stranger = p.env.keypair();
    let ix = p.update_provider_p2p_key_ix(&stranger.pubkey(), id, new);
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);
    let ix = p.update_provider_p2p_key_ix(&authority.pubkey(), id, [0u8; 32]);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidP2pKey);
    p.eject_provider(index);
    let ix = p.update_provider_p2p_key_ix(&authority.pubkey(), id, old);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::ProviderRemoved);
}

/// As at registration, the endpoint's variant names the kind and must match the one account
/// passed.
#[test]
fn an_endpoint_update_must_match_the_account_it_names() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("endpoint-kind"));
    let provider = p.providers[index].pda;
    let authority = p.providers[index].authority.insecure_clone();
    let node_owner = p.nodes[0].owner.insecure_clone();
    let node = p.nodes[0].pda;
    let p2p = p.p2p_key();
    let address = MemberEndpoint::Node {
        ip: [203, 0, 113, 9],
        port: 4_000,
    };

    // A node address for the provider, and a p2p key for the node.
    let ix = p.update_endpoint_raw_ix(&authority.pubkey(), None, Some(provider), address.clone());
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::MemberKindMismatch);
    let ix = p.update_endpoint_raw_ix(
        &node_owner.pubkey(),
        Some(node),
        None,
        MemberEndpoint::Provider { p2p_key: p2p },
    );
    let failed = p.env.send_err(&[ix], &[&node_owner]);
    assert_molpha_error(&failed, MolphaError::MemberKindMismatch);

    // Neither account.
    let ix = p.update_endpoint_raw_ix(&authority.pubkey(), None, None, address);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::ExactlyOneMember);

    // A node owner cannot move a provider, nor a provider authority a node.
    let ix = p.update_endpoint_raw_ix(
        &node_owner.pubkey(),
        None,
        Some(provider),
        MemberEndpoint::Provider { p2p_key: p2p },
    );
    let failed = p.env.send_err(&[ix], &[&node_owner]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);
    assert_eq!(
        p.provider_account(p.providers[index].id).p2p_key,
        p.providers[index].p2p_key
    );
}

// ---------------------------------------------------------------------------
// key rotation: `rotate_node_key` serves providers too
// ---------------------------------------------------------------------------

const WINDOW_SECONDS: i64 = 604_800;

#[test]
fn a_provider_rotates_its_key_at_the_same_record_and_nothing_else_moves() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("prot-a"));
    let index = p.add_active_provider(fresh_key("prot-b"));
    let old_key = p.providers[index].key.clone();
    let new_key = fresh_key("prot-b-new");
    let (id, pda_b) = (p.providers[index].id, p.providers[index].pda);
    let record = PROVIDER_BIT_BASE + 2;
    let before = p.current_version();
    let before_snapshot = p.env.registry(before);

    let meta = p.rotate_provider_key(index, new_key.clone());
    let now = p.env.unix_timestamp();

    assert_eq!(p.current_version(), before + 1);
    let after = p.env.registry(before + 1);
    assert_eq!(
        after.nodes, before_snapshot.nodes,
        "same members, same records"
    );
    for bit in 0..after.nodes.len() {
        let expected = if bit == record {
            (new_key.x, new_key.y)
        } else {
            before_snapshot.keys[bit]
        };
        assert_eq!(after.keys[bit], expected, "record {bit}");
    }
    // The replaced version still names the old key, for its grace window.
    let replaced = p.env.registry(before);
    assert_eq!(replaced.keys[record], (old_key.x, old_key.y));
    assert_eq!(replaced.grace_active_until, now + WINDOW_SECONDS);

    let account = p.provider_account(id);
    assert_eq!(account.xy(), (new_key.x, new_key.y));
    assert_eq!(account.key_rotated_at, now);
    assert_eq!(account.status, ProviderStatus::Active);
    assert_eq!(account.slot, 2);
    assert_eq!(p.provider_account(p.providers[a].id).key_rotated_at, 0);

    // Both keys stay reserved for the provider.
    for key in [&new_key, &old_key] {
        let claim: KeyXClaim = p.env.get(&pda::key_x_claim(&key.x));
        assert_eq!(claim.owner, pda_b);
    }

    let event: ProviderKeyRotated = only_event(&meta);
    assert_eq!((event.id, event.slot), (id, 2));
    assert_eq!(event.authority, p.providers[index].authority.pubkey());
    assert_eq!(event.registry_version, before + 1);
    assert_eq!(
        (event.old_pubkey_x, event.new_pubkey_x),
        (old_key.x, new_key.x)
    );
    assert_eq!(event.rotated_at, now);
}

#[test]
fn the_rotated_key_signs_provider_rounds() {
    let mut p = protocol();
    let provider = p.add_active_provider(fresh_key("rot-sign"));
    let new_key = fresh_key("rot-sign-new");
    p.rotate_provider_key(provider, new_key.clone());

    // The current version names the new key, so a round signed with it verifies.
    let duty = live(&p, 3);
    let signed = with_provider(&p, provider, duty);
    submit(&mut p, &signed);
    let registry = p.env.registry(p.current_version());
    assert_eq!(
        registry.keys[p.providers[provider].bit()],
        (new_key.x, new_key.y)
    );
}

#[test]
fn a_provider_rotates_at_most_once_per_settlement_window() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("prot-cool"));
    let authority = p.providers[index].authority.insecure_clone();
    let second = fresh_key("prot-cool-2");

    p.rotate_provider_key(index, fresh_key("prot-cool-1"));
    let rotated_at = p.env.unix_timestamp();
    let version = p.current_version();

    for wait in [0, WINDOW_SECONDS - 1] {
        p.env.warp_to(rotated_at + wait);
        let proof = p.provider_key_rotation_proof(index, &second);
        let ix = p.rotate_provider_key_ix(index, proof);
        let failed = p.env.send_err(&[ix], &[&authority]);
        assert_molpha_error(&failed, MolphaError::KeyRotationCooldownActive);
        assert_eq!(p.current_version(), version);
    }
    p.env.warp_to(rotated_at + WINDOW_SECONDS);
    p.rotate_provider_key(index, second);
    assert_eq!(p.current_version(), version + 1);
}

#[test]
fn only_an_active_provider_can_rotate() {
    let mut p = protocol();
    let pending = p.register_provider(fresh_key("prot-pending"));
    let authority = p.providers[pending].authority.insecure_clone();
    let new_key = fresh_key("prot-pending-new");

    p.eject_provider(pending);
    let proof = p.provider_key_rotation_proof(pending, &new_key);
    let ix = p.rotate_provider_key_ix(pending, proof);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::ProviderNotActive);
    assert!(!p.env.account_exists(&pda::key_x_claim(&new_key.x)));
}

#[test]
fn a_provider_rotation_needs_its_authority_and_its_current_key() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("prot-auth"));
    let id = p.providers[index].id;
    let authority = p.providers[index].authority.insecure_clone();
    let new_key = fresh_key("prot-auth-new");
    let version = p.current_version();

    // A stranger with a valid proof.
    let stranger = p.env.keypair();
    let proof = p.provider_key_rotation_proof(index, &new_key);
    let ix = p.rotate_provider_key_raw_ix(&stranger.pubkey(), id, proof);
    let failed = p.env.send_err(&[ix], &[&stranger]);
    assert_molpha_error(&failed, MolphaError::UnauthorizedNodeAuthority);

    // The authority, with a proof signed by some other key.
    let imposter = fresh_key("prot-auth-imposter");
    let proof =
        crypto::key_rotation_proof(&molpha::ID, &p.providers[index].pda, &imposter, &new_key);
    let ix = p.rotate_provider_key_ix(index, proof);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidKeyRotationSignature);

    // A proof made for another provider is no proof for this one: its PoP is over that
    // provider's digest.
    let other = p.add_active_provider(fresh_key("prot-auth-other"));
    let version = version + 1;
    let proof = p.provider_key_rotation_proof(other, &new_key);
    let ix = p.rotate_provider_key_raw_ix(&authority.pubkey(), id, proof);
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert_molpha_error(&failed, MolphaError::InvalidPoP);

    assert_eq!(p.current_version(), version);
    assert!(!p.env.account_exists(&pda::key_x_claim(&new_key.x)));
}

#[test]
fn a_provider_and_a_node_cannot_use_each_others_account_slot() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("prot-slot"));
    let authority = p.providers[index].authority.insecure_clone();
    let new_key = fresh_key("prot-slot-new");
    let proof = p.provider_key_rotation_proof(index, &new_key);
    let version = p.current_version();

    let mut accounts = molpha::accounts::RotateNodeKey {
        node_owner: authority.pubkey(),
        protocol_config: pda::protocol_config(),
        registry_state: pda::registry_state(),
        current_registry: pda::registry(version),
        new_registry: pda::registry(version + 1),
        node: None,
        provider: Some(p.providers[index].pda),
        key_x_claim: pda::key_x_claim_for(&proof.new_public_key_compressed),
        system_program: anchor_lang::solana_program::system_program::ID,
    };
    let ix_with = |accounts: &molpha::accounts::RotateNodeKey| {
        let mut ix = p.rotate_provider_key_ix(index, proof.clone());
        ix.accounts = anchor_lang::ToAccountMetas::to_account_metas(accounts, None);
        ix
    };

    // Neither.
    accounts.provider = None;
    let failed = p.env.send_err(&[ix_with(&accounts)], &[&authority]);
    assert_molpha_error(&failed, MolphaError::ExactlyOneMember);

    // Both: the authority also owns a node here, so both accounts pass their own checks.
    let node_owner = p.nodes[0].owner.insecure_clone();
    let node_proof = p.key_rotation_proof(0, &new_key);
    accounts.node_owner = node_owner.pubkey();
    accounts.node = Some(p.nodes[0].pda);
    accounts.provider = Some(p.providers[index].pda);
    accounts.key_x_claim = pda::key_x_claim_for(&node_proof.new_public_key_compressed);
    let mut ix = p.rotate_node_key_ix(0, node_proof);
    ix.accounts = anchor_lang::ToAccountMetas::to_account_metas(&accounts, None);
    let failed = p.env.send_err(&[ix], &[&node_owner]);
    // The provider's own authority check fails first when the signer is not its authority.
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("UnauthorizedNodeAuthority") || l.contains("ExactlyOneMember")),
        "{:?}",
        failed.meta.logs
    );
    assert_eq!(p.current_version(), version);
}

#[test]
fn a_provider_cannot_rotate_to_a_nodes_key_or_its_negation() {
    let mut p = protocol();
    let index = p.add_active_provider(fresh_key("prot-dup"));
    let authority = p.providers[index].authority.insecure_clone();
    let node_key = p.nodes[0].key.clone();

    for taken in [node_key.clone(), negated(&node_key)] {
        let proof = p.provider_key_rotation_proof(index, &taken);
        let ix = p.rotate_provider_key_ix(index, proof);
        let failed = p.env.send_err(&[ix], &[&authority]);
        assert!(failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")));
    }
}

// ---------------------------------------------------------------------------
// attestations with a provider bit
// ---------------------------------------------------------------------------

fn submit(p: &mut Protocol, signed: &molpha_tests::scenario::SignedAttestation) -> Keypair {
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), signed);
    p.env.send_ok(&[ix], &[&submitter]);
    submitter
}

/// A provider round under a live duty: `required - 1` nodes of the round's own committee, plus the
/// provider.
fn with_provider(
    p: &Protocol,
    provider: usize,
    duty: Duty,
) -> molpha_tests::scenario::SignedAttestation {
    let bits = p.provider_selection_bits(&duty);
    let k = usize::from(duty.signatures_required) - 1;
    p.sign_duty_with_providers(duty, &bits[..k], &[provider])
}

#[test]
fn an_attestation_with_a_provider_bit_verifies_and_stores_the_bitmap() {
    let mut p = protocol();
    let provider = p.add_active_provider(fresh_key("att-ok"));
    let duty = live(&p, 3);
    let signed = with_provider(&p, provider, duty);
    let bit = p.providers[provider].bit();
    assert_eq!(bit, PROVIDER_BIT_BASE + 1);
    assert!(crypto::bitmap_bits(&signed.signers_bitmap).contains(&bit));

    let submitter = submit(&mut p, &signed);
    let feed: Feed = p
        .env
        .get(&pda::feed(&duty.source_id, 3, &submitter.pubkey()));
    assert_eq!(feed.signers_bitmap, signed.signers_bitmap);
}

#[test]
fn a_provider_counts_toward_the_quorum_and_a_second_one_is_refused() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("quorum-a"));
    let b = p.add_active_provider(fresh_key("quorum-b"));
    let duty = live(&p, 4);
    let bits = p.provider_selection_bits(&duty);
    let submitter = p.env.keypair();

    // Two nodes and the provider are three signers for a quorum of four: one short.
    let signed = p.sign_duty_with_providers(duty, &bits[..2], &[a]);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);

    // A second provider does not make up the difference: four bits, but the bitmap is refused.
    let signed = p.sign_duty_with_providers(duty, &bits[..2], &[a, b]);
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidSignersBitmap);

    // Three nodes and the provider are four signers.
    let signed = p.sign_duty_with_providers(duty, &bits[..3], &[a]);
    submit(&mut p, &signed);
}

#[test]
fn a_provider_only_bitmap_is_refused() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("only-a"));
    let duty = live(&p, 3);
    let signed = p.sign_duty_with_providers(duty, &[], &[a]);
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);
}

#[test]
fn a_provider_does_not_use_up_the_redundancy_buffer() {
    let mut p = protocol();
    let provider = p.add_active_provider(fresh_key("buffer"));
    let duty = live(&p, 3);
    // A provider round of quorum 3 has a node quorum of 2; the buffer of 2 allows four node bits,
    // and the provider rides along on top of them.
    let nodes = p.provider_selection_bits(&duty);
    assert_eq!(nodes.len(), 4);
    let signed = p.sign_duty_with_providers(duty, &nodes, &[provider]);
    submit(&mut p, &signed);

    // A fifth node bit is still too many, provider or not.
    let fifth = (0..NODE_COUNT).find(|bit| !nodes.contains(bit)).unwrap();
    let mut bits = nodes.clone();
    bits.push(fifth);
    let duty = duty.source_id([0x77; 32]);
    let signed = p.sign_duty_with_providers(duty, &bits, &[provider]);
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::CreditedExceedsSelection);
}

#[test]
fn a_provider_bit_must_name_a_live_record_and_be_in_the_signed_sum() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("sum-a"));
    let duty = live(&p, 3);
    let nodes = p.selection_bits(&duty)[..3].to_vec();
    let bit = p.providers[a].bit();
    let submitter = p.env.keypair();

    // The nodes signed; the provider did not. Setting its bit afterwards breaks the signature.
    let mut signed = p.sign_duty_with_providers(duty, &nodes, &[]);
    signed.signers_bitmap = crypto::bitmap_from_bits(&[nodes.as_slice(), &[bit]].concat());
    let mut args = signed.submit_args();
    args.coalition_key = p.coalition_key_for(&args.attestation);
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidAggregateSignature);

    // Bits on records that are holes: the reserved slot 0, a slot never taken, one past the table.
    for hole in [
        PROVIDER_BIT_BASE,
        PROVIDER_BIT_BASE + 2,
        PROVIDER_BIT_BASE + 40,
        255,
    ] {
        let mut signed = p.sign_duty_with_providers(duty, &nodes, &[a]);
        signed.signers_bitmap = crypto::bitmap_from_bits(&[nodes.as_slice(), &[hole]].concat());
        let mut args = signed.submit_args();
        args.coalition_key = molpha_tests::scenario::ZERO_COALITION_KEY;
        let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
        let failed = p.env.send_err(&[ix], &[&submitter]);
        assert_molpha_error(&failed, MolphaError::InvalidSignersBitmap);
    }
}

#[test]
fn an_ejected_provider_bit_is_refused_in_the_new_version_and_honoured_in_the_old_within_grace() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("eject-grace"));
    let old_version = p.current_version();
    let duty = Duty::new(old_version, 3).at_secs(p.env.unix_timestamp() as u64);
    let signed_old = with_provider(&p, a, duty);

    p.eject_provider(a);
    let new_version = p.current_version();
    assert_eq!(new_version, old_version + 1);

    // The old version stays verifiable for its grace window: a round it served still lands.
    submit(&mut p, &signed_old);

    // A new round cannot name the ejected slot: the record is a hole.
    let duty = Duty::new(new_version, 3).at_secs(p.env.unix_timestamp() as u64);
    let nodes = p.selection_bits(&duty)[..3].to_vec();
    let mut signed = p.sign_duty_with_bits(duty, &nodes, None);
    signed.signers_bitmap =
        crypto::bitmap_from_bits(&[nodes.as_slice(), &[PROVIDER_BIT_BASE + 1]].concat());
    let mut args = signed.submit_args();
    args.coalition_key = molpha_tests::scenario::ZERO_COALITION_KEY;
    let submitter = p.env.keypair();
    let ix = p.submit_attestation_ix_with_args(&submitter.pubkey(), args);
    let failed = p.env.send_err(&[ix], &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidSignersBitmap);
}

#[test]
fn a_reused_slot_resolves_to_the_key_that_held_it_in_the_attestations_version() {
    let mut p = protocol();
    let a = p.add_active_provider(fresh_key("reuse-a"));
    let duty_a = Duty::new(p.current_version(), 3).at_secs(p.env.unix_timestamp() as u64);
    let signed_a = with_provider(&p, a, duty_a);

    p.eject_provider(a);
    let b = p.add_active_provider(fresh_key("reuse-b"));
    assert_eq!(p.providers[b].slot, p.providers[a].slot);

    // Version-resolved: the old attestation names the version in which slot 1 was `a`.
    submit(&mut p, &signed_a);
    // And the new holder signs under the new version.
    let duty_b = Duty::new(p.current_version(), 3)
        .source_id([0x55; 32])
        .at_secs(p.env.unix_timestamp() as u64);
    let signed_b = with_provider(&p, b, duty_b);
    submit(&mut p, &signed_b);
}

// ---------------------------------------------------------------------------
// tickets and selection ignore provider bits
// ---------------------------------------------------------------------------

#[test]
fn selection_is_the_same_with_providers_in_the_registry() {
    let mut p = protocol();
    p.add_active_providers("selection", 3);
    let registry = p.env.registry(p.current_version());
    assert_eq!(
        registry.provider_count, 4,
        "slot 0 reserved, then three providers"
    );

    // The same snapshot with its provider table cut away: the nodes alone.
    let without = molpha_verifier::RegistryView {
        provider_count: 0,
        providers: &[],
        ..registry.selection_view()
    };
    for i in 0..32u8 {
        let duty = Duty::new(registry.version, 3).source_id([i; 32]);
        let derive = |view: &molpha_verifier::RegistryView| {
            molpha_verifier::derive_selection_bitmap(
                &duty.source_id,
                duty.timestamp,
                duty.signatures_required,
                view,
            )
            .unwrap()
            .to_bytes()
        };
        let with = derive(&registry.selection_view());
        assert_eq!(with, derive(&without), "source {i}");
        assert!(p.selection_bits(&duty).iter().all(|bit| *bit < NODE_SLOTS));
    }
}

#[test]
fn a_ticket_credits_only_nodes() {
    // Activating the provider waits out its delay; epochs that still cover the clock afterwards.
    let mut p = Protocol::builder()
        .epoch_len_seconds(molpha::state::MAX_EPOCH_LEN_SECONDS)
        .start();
    p.register_nodes("ticket-nodes", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("ticket-provider"));
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let duty = p.live_duty(3);
    // A provider round of quorum 3: two nodes of its own draw (2 + buffer) and the provider.
    let selection = p.provider_selection_bits(&duty);
    let signed = p.sign_duty_with_providers(duty, &selection[..2], &[provider]);

    p.submit_ticket(&submitter, &gateway, &signed);

    let epoch = p.open_epoch();
    let state = p.env.epoch(epoch);
    assert_eq!(state.tickets, 1);
    // Every selected node is counted as selected; only the two that signed as signed; the
    // provider is in no counter and not in the roster.
    assert_eq!(state.sum_selected as usize, selection.len());
    assert_eq!(state.sum_signed, 2);
    assert_eq!(usize::from(state.roster_len), selection.len());
    let provider_pda = p.providers[provider].pda.to_bytes();
    assert!(state.roster[..usize::from(state.roster_len)]
        .iter()
        .all(|key| *key != provider_pda));
}

// ---------------------------------------------------------------------------
// slashing sees nodes only
// ---------------------------------------------------------------------------

#[test]
fn a_provider_riding_along_is_not_an_unsolicited_signer() {
    let mut p = Protocol::start();
    p.register_nodes("slash-unsolicited", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("slash-unsolicited-provider"));
    let reporter = p.reporter();
    let duty = Duty::new(p.current_version(), 4).at_secs(p.env.unix_timestamp() as u64);
    let selection = p.provider_selection_bits(&duty);

    // Everyone selected for the provider round plus the provider: nobody is unsolicited.
    let signed = p.sign_duty_with_providers(duty, &selection, &[provider]);
    let ix = p.slash_unsolicited_signing_ix(
        &reporter.pubkey(),
        molpha::state::UnsolicitedSigningEvidence {
            attestation: signed.attestation(),
            coalition_key: signed.coalition_key,
        },
    );
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersFullySelected);

    // With an intruder node beside the provider, only the node is guilty.
    let duty = duty.source_id([0x21; 32]);
    let selection = p.provider_selection_bits(&duty);
    let intruder = (0..NODE_COUNT)
        .find(|bit| !selection.contains(bit))
        .unwrap();
    let mut bits = selection[..3].to_vec();
    bits.push(intruder);
    let signed = p.sign_duty_with_providers(duty, &bits, &[provider]);
    let intruder_pda = p.env.registry(signed.duty.registry_version).nodes[intruder];
    let ix = p.slash_unsolicited_signing_ix(
        &reporter.pubkey(),
        molpha::state::UnsolicitedSigningEvidence {
            attestation: signed.attestation(),
            coalition_key: signed.coalition_key,
        },
    );
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let proven: molpha::events::OffenceProven = only_event(&meta);
    assert_eq!(proven.guilty_bitmap, crypto::bitmap_from_bits(&[intruder]));
    assert_eq!(
        p.env.get::<molpha::state::Node>(&intruder_pda).status,
        NodeStatus::Tombstoned
    );
    assert_eq!(
        p.provider_account(p.providers[provider].id).status,
        ProviderStatus::Active
    );
}

#[test]
fn equivocation_punishes_the_shared_nodes_and_never_a_shared_provider() {
    let mut p = Protocol::start();
    p.register_nodes("slash-equivocation", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("slash-equivocation-provider"));
    let reporter = p.reporter();
    let duty = Duty::new(p.current_version(), 4)
        .source_id([0x5a; 32])
        .at_secs(p.env.unix_timestamp() as u64);
    let selection = p.selection_bits(&duty);

    // Two values for one duty: nodes 0..3 and 2..5 of the selection, the provider in both.
    let a = p.sign_duty_with_providers(duty.value([1; 32]), &selection[0..4], &[provider]);
    let b = p.sign_duty_with_providers(duty.value([2; 32]), &selection[2..5], &[provider]);
    let evidence = molpha::state::EquivocationEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    };
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let proven: molpha::events::OffenceProven = only_event(&meta);
    assert_eq!(
        proven.guilty_bitmap,
        crypto::bitmap_from_bits(&selection[2..4])
    );
    assert_eq!(
        p.provider_account(p.providers[provider].id).status,
        ProviderStatus::Active
    );
    let registry = p.env.registry(p.current_version());
    assert!(registry.is_live(p.providers[provider].bit()));

    // Two attestations with disjoint nodes that share only the provider prove no node guilty.
    let duty = duty.source_id([0x5b; 32]);
    let selection = p.selection_bits(&duty);
    let a = p.sign_duty_with_providers(duty.value([1; 32]), &selection[0..2], &[provider]);
    let b = p.sign_duty_with_providers(duty.value([2; 32]), &selection[2..4], &[provider]);
    let evidence = molpha::state::EquivocationEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    };
    let ix = p.slash_equivocation_ix(&reporter.pubkey(), evidence);
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::GuiltySetEmpty);
}

#[test]
fn nonce_reuse_and_inactivity_evidence_cannot_name_a_provider_index() {
    let mut p = Protocol::start();
    p.register_nodes("slash-index", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("slash-index-provider"));
    let reporter = p.reporter();
    let bit = p.providers[provider].bit() as u32;
    let version = p.current_version();

    // Nonce reuse: the node index check comes before any proof is looked at.
    let transcript = |b: u8| molpha::state::PartialTranscript {
        b: [b; 32],
        c: [b; 32],
        s: [b; 32],
    };
    let evidence = molpha::state::NonceReuseEvidence {
        registry_version: version,
        node_index: bit,
        epoch: 0,
        batch_meta: [0; 32],
        batch_root: [0; 32],
        batch_sig_s: [1; 32],
        batch_commitment_addr: [0; 20],
        slot: 0,
        nonces: molpha::state::CommittedNoncePair {
            r1_x: [0; 32],
            r1_y: [0; 32],
            r2_x: [0; 32],
            r2_y: [0; 32],
        },
        merkle_proof: Vec::new(),
        transcript_a: transcript(1),
        transcript_b: transcript(2),
    };
    let ix = p.slash_nonce_reuse_ix(&reporter.pubkey(), evidence);
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeIndex);

    // Inactivity: likewise, with the accounts it asks for in place.
    let evidence = molpha::state::InactivityEvidence {
        epoch: 0,
        roster_slot: 0,
        registry_version: version,
        node_index: bit,
    };
    let ix = p.slash_offence_ix(
        &reporter.pubkey(),
        molpha::state::Offence::Inactivity(evidence),
    );
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::InvalidNodeIndex);
}

#[test]
fn commitment_reuse_punishes_the_shared_nodes_and_never_the_provider() {
    const SHARED_NONCE: [u8; 32] = [0x5c; 32];
    let mut p = Protocol::start();
    p.register_nodes("slash-commitment", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("slash-commitment-provider"));
    let reporter = p.reporter();
    let now = p.env.unix_timestamp() as u64;
    let a_duty = Duty::new(p.current_version(), 4)
        .source_id([0x11; 32])
        .at_secs(now);
    let selection_a = p.provider_selection_bits(&a_duty);
    // Provider rounds of quorum 4 draw 3 + buffer nodes: find a second duty whose draw shares at
    // least the three nodes the coalition needs.
    let (b_duty, shared) = (0x22u8..=0xff)
        .find_map(|i| {
            let b_duty = Duty::new(p.current_version(), 4)
                .source_id([i; 32])
                .value([0xbb; 32])
                .at_secs(now);
            let selection_b = p.provider_selection_bits(&b_duty);
            let shared: Vec<usize> = selection_a
                .iter()
                .copied()
                .filter(|bit| selection_b.contains(bit))
                .collect();
            (shared.len() >= 3).then_some((b_duty, shared))
        })
        .expect("two duties whose provider draws share three nodes");

    // One coalition, provider included, signing two duties under one nonce: three nodes and the
    // provider are the four signers of the quorum.
    let mut bits = shared[..3].to_vec();
    bits.push(p.providers[provider].bit());
    let a = p.sign_duty_with_bits(a_duty, &bits, Some(SHARED_NONCE));
    let b = p.sign_duty_with_bits(b_duty, &bits, Some(SHARED_NONCE));
    assert_eq!(a.signature.commitment, b.signature.commitment);

    let evidence = molpha::state::CommitmentReuseEvidence {
        attestation_a: a.attestation(),
        attestation_b: b.attestation(),
        coalition_key_a: a.coalition_key,
        coalition_key_b: b.coalition_key,
    };
    let ix = p.slash_commitment_reuse_ix(&reporter.pubkey(), evidence);
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let proven: molpha::events::OffenceProven = only_event(&meta);
    assert_eq!(proven.guilty_bitmap, crypto::bitmap_from_bits(&shared[..3]));
    assert_eq!(
        p.provider_account(p.providers[provider].id).status,
        ProviderStatus::Active
    );
    assert!(p
        .env
        .registry(p.current_version())
        .is_live(p.providers[provider].bit()));
}

#[test]
fn a_ticket_with_too_few_signers_is_refused_and_a_second_provider_never_pads_it() {
    let mut p = Protocol::builder()
        .epoch_len_seconds(molpha::state::MAX_EPOCH_LEN_SECONDS)
        .start();
    p.register_nodes("ticket-short", NODE_COUNT);
    let providers = p.add_active_providers("ticket-short-providers", 2);
    let gateway = p.register_gateway();
    let submitter = p.env.keypair();
    let duty = p.live_duty(4);
    let selection = p.provider_selection_bits(&duty);

    // Two nodes and the provider are three signers for a quorum of four.
    let signed = p.sign_duty_with_providers(duty, &selection[..2], &providers[..1]);
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::QuorumBelowThreshold);

    // A second provider makes four bits, and the bitmap is refused instead of counted.
    let signed = p.sign_duty_with_providers(duty, &selection[..2], &providers);
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &signed);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::InvalidSignersBitmap);
    assert_eq!(p.env.epoch(p.open_epoch()).tickets, 0);
}

/// The draw of `required - 1` nodes is not inside the draw of `required`: a provider round must be
/// ticketed and policed against its own draw, or the honest nodes of a provider round would read
/// as unsolicited and the committee's as selected.
#[test]
fn a_provider_round_is_policed_against_its_own_node_quorum_draw() {
    let mut p = Protocol::builder()
        .epoch_len_seconds(molpha::state::MAX_EPOCH_LEN_SECONDS)
        .start();
    p.register_nodes("policing", NODE_COUNT);
    let provider = p.add_active_provider(fresh_key("policing-provider"));
    let gateway = p.register_gateway();
    let reporter = p.reporter();
    let submitter = p.env.keypair();
    let base = p.live_duty(3);

    // A duty where the draws differ both ways: `only_provider` is selected by the provider round
    // alone, `only_committee` by the committee round alone.
    let (duty, only_provider, only_committee) = (0u8..=255)
        .find_map(|i| {
            let duty = base.source_id([i; 32]);
            let committee = p.selection_bits(&duty);
            let provider_draw = p.provider_selection_bits(&duty);
            let x = provider_draw
                .iter()
                .copied()
                .find(|b| !committee.contains(b))?;
            let y = committee
                .iter()
                .copied()
                .find(|b| !provider_draw.contains(b))?;
            Some((duty, x, y))
        })
        .expect("a source whose draws differ both ways");
    let provider_draw = p.provider_selection_bits(&duty);
    let partner = provider_draw
        .iter()
        .copied()
        .find(|b| *b != only_provider)
        .unwrap();
    let honest = p.sign_duty_with_providers(duty, &[only_provider, partner], &[provider]);
    let intruder = p.sign_duty_with_providers(duty, &[only_committee, partner], &[provider]);

    // A ticket of the committee's node is refused: it is outside the provider round's draw.
    let ixs = p.ticket_ixs(&submitter.pubkey(), &gateway, &intruder);
    let failed = p.env.send_err(&ixs, &[&submitter]);
    assert_molpha_error(&failed, MolphaError::SignersNotSubsetOfSelection);

    // The provider round's own node is accepted, although the committee round did not select it.
    p.submit_ticket(&submitter, &gateway, &honest);

    // Unsolicited-signing evidence agrees: nobody signed outside the provider round's draw ...
    let filing = |signed: &molpha_tests::scenario::SignedAttestation| {
        molpha::state::UnsolicitedSigningEvidence {
            attestation: signed.attestation(),
            coalition_key: signed.coalition_key,
        }
    };
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), filing(&honest));
    let failed = p.env.send_err(&[ix], &[&reporter]);
    assert_molpha_error(&failed, MolphaError::SignersFullySelected);

    // ... except the committee's node, which is the only one guilty.
    let ix = p.slash_unsolicited_signing_ix(&reporter.pubkey(), filing(&intruder));
    let meta = p.env.send_ok(&[ix], &[&reporter]);
    let proven: molpha::events::OffenceProven = only_event(&meta);
    assert_eq!(
        proven.guilty_bitmap,
        crypto::bitmap_from_bits(&[only_committee])
    );
}
