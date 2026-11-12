//! Native fuzz oracles. Inputs model untrusted bytes or bounded, internally valid epoch states.
//! A panic is a candidate requiring reachability analysis, not automatically an on-chain exploit.

use anchor_lang::prelude::{AccountInfo, Pubkey};
use anchor_lang::{AccountDeserialize, AnchorDeserialize, AnchorSerialize, Discriminator};
use molpha::instructions::{
    AddDelegateArgs, InitializeArgs, MemberEndpoint, RegisterGatewayArgs, RegisterNodeArgs,
    RemoveDelegateArgs, SubmitAttestationArgs, UpdateConfigArgs,
};
use molpha::state::{
    CommitmentReuseEvidence, EpochState, EquivocationEvidence, InactivityEvidence, Node,
    NodeStatus, NonceReuseEvidence, Offence, PlanConfig, ProtocolConfig, Registry, RegistryMember,
    RegistryRef, UnsolicitedSigningEvidence, TAU_MIN,
};
use molpha_verifier::{Attestation, CoalitionKey, KeyRotationProof};

fn exercise_registry(registry: &RegistryRef<'_>) {
    assert!(registry.records().len() <= 256);
    assert!(registry.nodes().len() <= 64);
    assert_eq!(
        registry.live_count(),
        registry
            .records()
            .iter()
            .filter(|member| !member.is_vacant())
            .count()
    );
    assert_eq!(
        registry.live_node_count(),
        registry
            .nodes()
            .iter()
            .filter(|member| !member.is_vacant())
            .count()
    );
    let _ = registry.providers();
    let _ = registry.first_free_provider_slot();
    let view = registry.view();
    assert_eq!(usize::from(view.node_count), registry.nodes().len());
    assert_eq!(usize::from(view.provider_count), registry.providers().len());
    for bit in 0..256 {
        let member = registry.member(bit);
        let node = registry.node(bit);
        assert_eq!(member.is_ok(), node.is_ok());
        if let Ok(member) = member {
            let key = Pubkey::new_from_array(member.node);
            assert_eq!(registry.get_index(key).unwrap(), bit);
            assert!(registry.is_node_in_registry(key));
            let _ = member.xy();
        }
    }
    assert!(registry.get_index(Pubkey::default()).is_err());
    assert!(!registry.is_node_in_registry(Pubkey::default()));
}

fn constructed_registry(input: &[u8]) -> Vec<u8> {
    let node_count = input.first().copied().unwrap_or(0) % 5;
    let provider_count = input.get(1).copied().unwrap_or(0) % 5;
    let records = usize::from(node_count) + usize::from(provider_count);
    let mut data = vec![0; Registry::space(records)];
    data[..8].copy_from_slice(Registry::DISCRIMINATOR);
    let mut header: Registry = bytemuck::Zeroable::zeroed();
    header.version = 7;
    header.node_count = node_count;
    header.provider_count = provider_count;
    header.redundancy_buffer = 2;
    data[8..Registry::HEADER_SPACE].copy_from_slice(bytemuck::bytes_of(&header));
    let table: &mut [RegistryMember] =
        bytemuck::cast_slice_mut(&mut data[Registry::HEADER_SPACE..]);
    for (index, member) in table.iter_mut().enumerate() {
        let live = input.get(index + 2).copied().unwrap_or(index as u8) & 1 == 1;
        if live {
            let mut key = [0; 32];
            key[0] = index as u8 + 1;
            *member =
                RegistryMember::new(Pubkey::new_from_array(key), ([index as u8; 32], [1; 32]));
        }
    }
    data
}

pub fn registry_bytes(input: &[u8]) {
    // Each control bit changes one trust boundary independently; preserve arbitrary payload bytes.
    let Some((&control, payload)) = input.split_first() else {
        return;
    };
    let key = Pubkey::default();
    let wrong_owner = Pubkey::new_from_array([7; 32]);
    let owner = if control & 1 == 0 {
        &molpha::ID
    } else {
        &wrong_owner
    };
    let mut lamports = 1_000_000;
    let mut data = payload.to_vec();
    let ai = AccountInfo::new(&key, false, false, &mut lamports, &mut data, owner, false);
    if let Ok(registry) = RegistryRef::load(&ai) {
        assert_eq!(*owner, molpha::ID);
        exercise_registry(&registry);
    };

    // Valid snapshots with independently controlled node/provider holes ensure arbitrary-byte
    // fuzzing does not spend all of its time at the discriminator and shape checks.
    let mut lamports = 1_000_000;
    let mut data = constructed_registry(payload);
    let ai = AccountInfo::new(
        &key,
        false,
        false,
        &mut lamports,
        &mut data,
        &molpha::ID,
        false,
    );
    exercise_registry(&RegistryRef::load(&ai).expect("constructed registry is valid"));
}

fn canonical_round_trip<T: AnchorDeserialize + AnchorSerialize>(payload: &[u8]) {
    if let Ok(value) = T::try_from_slice(payload) {
        let encoded = borsh::to_vec(&value).expect("serialize accepted wire value");
        assert_eq!(payload, encoded);
        let decoded = T::try_from_slice(&encoded).expect("canonical encoding decodes");
        assert_eq!(borsh::to_vec(&decoded).unwrap(), encoded);
    }
}

pub fn wire_bytes(input: &[u8]) {
    let Some((&kind, payload)) = input.split_first() else {
        return;
    };
    match kind % 18 {
        0 => canonical_round_trip::<Attestation>(payload),
        1 => canonical_round_trip::<CoalitionKey>(payload),
        2 => canonical_round_trip::<KeyRotationProof>(payload),
        3 => canonical_round_trip::<InitializeArgs>(payload),
        4 => canonical_round_trip::<UpdateConfigArgs>(payload),
        5 => canonical_round_trip::<RegisterNodeArgs>(payload),
        6 => canonical_round_trip::<MemberEndpoint>(payload),
        7 => canonical_round_trip::<AddDelegateArgs>(payload),
        8 => canonical_round_trip::<RemoveDelegateArgs>(payload),
        9 => canonical_round_trip::<RegisterGatewayArgs>(payload),
        10 => canonical_round_trip::<SubmitAttestationArgs>(payload),
        11 => canonical_round_trip::<PlanConfig>(payload),
        12 => canonical_round_trip::<EquivocationEvidence>(payload),
        13 => canonical_round_trip::<UnsolicitedSigningEvidence>(payload),
        14 => canonical_round_trip::<CommitmentReuseEvidence>(payload),
        15 => canonical_round_trip::<NonceReuseEvidence>(payload),
        16 => canonical_round_trip::<InactivityEvidence>(payload),
        _ => {
            if let Ok(value) = Offence::try_from_slice(payload) {
                let encoded = borsh::to_vec(&value).expect("serialize offence");
                assert_eq!(payload, encoded);
                assert_eq!(value.hash(), value.evidence_hash());
                canonical_round_trip::<Offence>(&encoded);
            }
        }
    }
}

pub fn epoch_arithmetic(input: &[u8]) {
    if input.len() < 24 {
        return;
    }
    let pool = u64::from_le_bytes(input[0..8].try_into().unwrap());
    let raw_tau = u64::from_le_bytes(input[8..16].try_into().unwrap());
    let mut epoch: EpochState = bytemuck::Zeroable::zeroed();
    epoch.pool = pool;
    epoch.tau = raw_tau.max(TAU_MIN);
    epoch.tickets = u32::from(u16::from_le_bytes(input[16..18].try_into().unwrap()));
    epoch.min_availability_bps = u16::from_le_bytes(input[18..20].try_into().unwrap()) % 10_001;
    epoch.min_samples = u16::from(input[20]) + 1;
    let target = 1 + u32::from(u16::from_le_bytes(input[21..23].try_into().unwrap())) % 8_191;
    let tau = epoch.next_tau(target);
    assert!(tau >= TAU_MIN);
    assert!(u128::from(tau) <= u128::from(epoch.tau) * 8);
    assert!(tau >= epoch.tau / 8);
    let len = (input.len() - 24).min(256);
    epoch.roster_len = len as u16;
    for (slot, value) in input[24..].iter().take(len).enumerate() {
        epoch.selected[slot] = u16::from(*value) + 1;
        epoch.signed[slot] = u16::from(*value / 2);
        epoch.sum_selected += u32::from(epoch.selected[slot]);
        epoch.sum_signed += u32::from(epoch.signed[slot]);
    }
    epoch.finalize();
    let paid: u128 = (0..len)
        .map(|slot| u128::from(epoch.payout(epoch.weight(slot))))
        .sum();
    assert!(paid <= u128::from(pool));
    for slot in 0..len {
        let weight = epoch.weight(slot);
        assert!(weight <= u64::from(epoch.signed[slot]));
        if epoch.weight_total != 0 {
            let share = u128::from(pool) * u128::from(weight) / u128::from(epoch.weight_total);
            assert_eq!(u128::from(epoch.payout(weight)), share);
        }
    }

    let a = u64::from_le_bytes(input[0..8].try_into().unwrap());
    let b = u64::from_le_bytes(input[8..16].try_into().unwrap());
    let c = u64::from_le_bytes(input[16..24].try_into().unwrap());
    let mut config_bytes = vec![0; ProtocolConfig::SPACE];
    config_bytes[..8].copy_from_slice(ProtocolConfig::DISCRIMINATOR);
    let mut config = ProtocolConfig::try_deserialize(&mut config_bytes.as_slice()).unwrap();
    config.reward_liability = a;
    config.pool_liability = b;
    config.protocol_reserved = c;
    assert_eq!(
        config.committed().ok(),
        a.checked_add(b).and_then(|sum| sum.checked_add(c))
    );

    let mut node = Node {
        authority: Pubkey::default(),
        secp256k1_pubkey_x: [0; 32],
        secp256k1_pubkey_y: [0; 32],
        status: NodeStatus::Active,
        ip: [0; 4],
        port: 0,
        locked_amount: 0,
        claimable_rewards: a,
        registered_at: 0,
        withdrawable_slot: 0,
        frozen_until: 0,
        bump: 0,
        key_rotated_at: 0,
    };
    let expected = a.checked_add(b);
    assert_eq!(node.add_rewards(b).is_ok(), expected.is_some());
    assert_eq!(node.claimable_rewards, expected.unwrap_or(a));
}
