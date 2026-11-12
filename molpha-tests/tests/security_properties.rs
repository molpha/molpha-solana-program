//! Native properties: no VM and no SBF artifact required when this module is filtered.

use anchor_lang::{AccountDeserialize, Discriminator};
use molpha::instructions::evidence_filing_open;
use molpha::state::{bps_of, EpochState, Node, OffenceKind, TAU_MIN};
use molpha::utils::{require_fresh_timestamp, MAX_FUTURE_DRIFT_SECONDS};
use molpha_tests::security::{fee_split, payout};
use molpha_tests::security_oracles;
use proptest::prelude::*;

fn node(locked: u64, rewards: u64) -> Node {
    let mut bytes = vec![0; Node::SPACE];
    bytes[..8].copy_from_slice(Node::DISCRIMINATOR);
    let mut node = Node::try_deserialize(&mut bytes.as_slice()).unwrap();
    node.locked_amount = locked;
    node.claimable_rewards = rewards;
    node
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn fees_conserve_gross_and_round_down(gross in any::<u64>(), bps in 0u16..=10_000) {
        let (fee, pool) = fee_split(gross, bps);
        prop_assert_eq!(bps_of(gross, bps), fee);
        prop_assert_eq!(u128::from(fee) + u128::from(pool), u128::from(gross));
        let numerator = u128::from(gross) * u128::from(bps);
        prop_assert!(u128::from(fee) * 10_000 <= numerator);
        prop_assert!(numerator < (u128::from(fee) + 1) * 10_000);
    }

    #[test]
    fn penalties_conserve_each_victims_balance(locked in any::<u64>(), rewards in any::<u64>(), kind in 0usize..5) {
        let kinds = [OffenceKind::Equivocation, OffenceKind::UnsolicitedSigning,
            OffenceKind::CommitmentReuse, OffenceKind::NonceReuse, OffenceKind::Inactivity];
        let mut victim = node(locked, rewards);
        let result = kinds[kind].penalty().apply(&mut victim, 1_700_200_000).unwrap();
        prop_assert_eq!(u128::from(victim.locked_amount) + u128::from(result.slashed_locked), u128::from(locked));
        prop_assert_eq!(u128::from(victim.claimable_rewards) + u128::from(result.slashed_rewards), u128::from(rewards));
        if kind == 2 {
            prop_assert_eq!(result.slashed_locked, locked / 2);
            prop_assert_eq!(result.slashed_rewards, rewards / 2);
        }
        if kind == 4 { prop_assert_eq!((victim.locked_amount, victim.claimable_rewards), (locked, rewards)); }
    }

    #[test]
    fn proportional_payouts_never_create_tokens(pool in any::<u64>(), weights in prop::collection::vec(0u16..=u16::MAX, 0..=256)) {
        let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
        let mut epoch: EpochState = bytemuck::Zeroable::zeroed();
        epoch.pool = pool;
        epoch.weight_total = total;
        let mut paid = 0u128;
        for weight in weights {
            let expected = payout(pool, u64::from(weight), total);
            prop_assert_eq!(epoch.payout(u64::from(weight)), expected);
            paid += u128::from(expected);
        }
        prop_assert!(paid <= u128::from(pool));
    }

    #[test]
    fn threshold_is_bounded_and_monotone(tau in TAU_MIN..=u64::MAX, observed in 0u32..=65_535, target in 1u32..8_191) {
        let mut epoch: EpochState = bytemuck::Zeroable::zeroed();
        epoch.tau = tau;
        epoch.tickets = observed;
        let next = epoch.next_tau(target);
        prop_assert!(next >= TAU_MIN && next >= tau / 8);
        prop_assert!(u128::from(next) <= u128::from(tau) * 8);
        prop_assert!(next <= epoch.next_tau(target + 1));
        epoch.tickets = observed + 1;
        prop_assert!(epoch.next_tau(target) <= next);
    }

    #[test]
    fn ticket_lottery_uses_a_strict_big_endian_prefix(tau in any::<u64>(), prefix in any::<u64>(), tail in any::<[u8;24]>()) {
        let mut epoch: EpochState = bytemuck::Zeroable::zeroed();
        epoch.tau = tau;
        let mut hash = [0;32];
        hash[..8].copy_from_slice(&prefix.to_be_bytes());
        hash[8..].copy_from_slice(&tail);
        prop_assert_eq!(epoch.is_ticket(&hash), prefix < tau);
        hash[..8].copy_from_slice(&tau.to_be_bytes());
        prop_assert!(!epoch.is_ticket(&hash));
    }

    #[test]
    fn freshness_and_evidence_have_opposite_deadline_inclusivity(now in 1_000_000i64..2_000_000_000, window in 1u64..604_801) {
        let now_u = now as u64;
        prop_assert!(require_fresh_timestamp(now_u - window, now, window).is_ok());
        prop_assert!(require_fresh_timestamp(now_u - window - 1, now, window).is_err());
        prop_assert!(require_fresh_timestamp(now_u + MAX_FUTURE_DRIFT_SECONDS, now, window).is_ok());
        prop_assert!(require_fresh_timestamp(now_u + MAX_FUTURE_DRIFT_SECONDS + 1, now, window).is_err());
        let deadline = now + window as i64;
        prop_assert!(evidence_filing_open(now, window, deadline - 1));
        prop_assert!(!evidence_filing_open(now, window, deadline));
    }

    #[test]
    fn public_registry_loader_handles_untrusted_bytes(bytes in prop::collection::vec(any::<u8>(), 0..2_048)) {
        security_oracles::registry_bytes(&bytes);
    }

    #[test]
    fn public_wire_decoders_handle_untrusted_bytes(bytes in prop::collection::vec(any::<u8>(), 0..2_048)) {
        security_oracles::wire_bytes(&bytes);
    }

    #[test]
    fn valid_epoch_states_obey_arithmetic_invariants(bytes in prop::collection::vec(any::<u8>(), 24..280)) {
        security_oracles::epoch_arithmetic(&bytes);
    }
}

#[test]
fn availability_floor_equality_is_paid_and_one_less_is_not() {
    let mut epoch: EpochState = bytemuck::Zeroable::zeroed();
    epoch.min_samples = 4;
    epoch.min_availability_bps = 8_000;
    epoch.sum_signed = 100;
    epoch.sum_selected = 100;
    epoch.selected[0] = 5;
    epoch.signed[0] = 4;
    assert!(!epoch.below_availability_floor(0));
    assert_eq!(epoch.weight(0), 4);
    epoch.signed[0] = 3;
    assert!(epoch.below_availability_floor(0));
    assert_eq!(epoch.weight(0), 0);
}
