//! Timestamp freshness bounds: inclusive lower/upper windows, negative-clock
//! fail-closed behavior, and saturation when the receipt delay exceeds `now`.

use molpha::utils::{require_fresh_timestamp, MAX_FUTURE_DRIFT_SECONDS};

const NOW: i64 = 1_700_000_000;
const WINDOW: u64 = 300;

fn fresh(ts: u64, now: i64) -> bool {
    require_fresh_timestamp(ts, now, WINDOW).is_ok()
}

#[test]
fn boundaries_are_inclusive() {
    let now = NOW as u64;
    assert!(!fresh(now - WINDOW - 1, NOW));
    assert!(fresh(now - WINDOW, NOW));
    assert!(fresh(now, NOW));
    assert!(fresh(now + MAX_FUTURE_DRIFT_SECONDS, NOW));
    assert!(!fresh(now + MAX_FUTURE_DRIFT_SECONDS + 1, NOW));
}

#[test]
fn negative_clock_fails_closed() {
    assert!(!fresh(NOW as u64, -1));
    assert!(!fresh(MAX_FUTURE_DRIFT_SECONDS + 1, -1));
    assert!(fresh(0, -1));
}

#[test]
fn window_larger_than_now_saturates_to_genesis() {
    assert!(fresh(0, 100));
    assert!(fresh(100 + MAX_FUTURE_DRIFT_SECONDS, 100));
    assert!(!fresh(100 + MAX_FUTURE_DRIFT_SECONDS + 1, 100));
}
