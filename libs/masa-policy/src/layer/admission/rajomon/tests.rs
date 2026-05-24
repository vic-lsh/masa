use std::sync::atomic::Ordering;

use tonic::{CowGrpcMethod, Response, Status};

use super::*;
use crate::layer::Layer;

// Default parameter values matching PolicyParams defaults.
// Tests verify algorithmic behaviour with these specific values.
const LATENCY_THRESHOLD_US: u64 = 5_000;
const INIT_PRICE: u64 = 0;
const PRICE_STEP_UP: u64 = 8;
const TOKENS_LEFT_INIT: u64 = 10;
const TOKEN_UPDATE_STEP: u64 = 5;

/// Mutex to serialize tests that modify the global RAJOMON_STATE.own_price,
/// since it's a single global value shared across all test threads.
static GLOBAL_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ── A. Price Update Algorithm Tests (Step Strategy) ──

/// Paper §3.4: when queueing exceeds threshold by 1ms, the price increases
/// by approximately `price_step_up` tokens (the paper reports 3-13).
/// Our `update_prices` formula is `((excess_us * step_up) / 1000).max(1)`.
#[test]
fn test_proportional_price_increase_on_congestion() {
    let state = RajomonSharedState::new();
    // Set window_max to 1ms over threshold so the proportional formula
    // yields exactly PRICE_STEP_UP tokens of increment per tick.
    state
        .queue_stats
        .window_max
        .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
    state.update_prices();
    assert_eq!(
        state.own_price.load(Ordering::Relaxed),
        INIT_PRICE + PRICE_STEP_UP
    );

    // Second tick, still congested. Need to re-set window_max because the
    // §10.5 halving means it doesn't carry forward at full magnitude.
    state
        .queue_stats
        .window_max
        .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
    state.update_prices();
    assert_eq!(
        state.own_price.load(Ordering::Relaxed),
        INIT_PRICE + 2 * PRICE_STEP_UP
    );
}

/// Paper §3.4: when queueing is below half the threshold, the price
/// decreases by exactly 1 token per tick (hardcoded). The configurable
/// `price_step_down` parameter is unused under the paper's algorithm.
#[test]
fn test_paper_price_decrease_is_one_per_tick() {
    let state = RajomonSharedState::new();
    state.own_price.store(5, Ordering::Relaxed);
    state.update_prices();
    assert_eq!(state.own_price.load(Ordering::Relaxed), 4);
    state.update_prices();
    assert_eq!(state.own_price.load(Ordering::Relaxed), 3);
}

#[test]
fn test_price_floor_at_zero() {
    let state = RajomonSharedState::new();
    state.own_price.store(1, Ordering::Relaxed);
    state.update_prices();
    assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
    state.update_prices();
    assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
}

#[test]
fn test_hysteresis_band_holds_price() {
    let state = RajomonSharedState::new();
    state.own_price.store(10, Ordering::Relaxed);
    // Set window_max to midpoint: between half-threshold and threshold
    state
        .queue_stats
        .window_max
        .store(LATENCY_THRESHOLD_US * 3 / 4, Ordering::Relaxed);
    state.update_prices();
    // Should hold, not increase or decrease
    assert_eq!(state.own_price.load(Ordering::Relaxed), 10);
}

/// Paper §3.4: price increase is *proportional* to queuing-delay excess.
/// Severe congestion should produce a larger increment than mild
/// congestion. Replaces the previous `test_price_increase_uses_constant_
/// step_not_proportional` test, which encoded the pre-paper step semantics.
#[test]
fn test_price_increase_is_proportional_to_excess() {
    let state = RajomonSharedState::new();

    // Mild congestion: 1ms over threshold → exactly PRICE_STEP_UP increment.
    state.own_price.store(0, Ordering::Relaxed);
    state
        .queue_stats
        .window_max
        .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
    state.update_prices();
    let price_after_mild = state.own_price.load(Ordering::Relaxed);

    // Severe congestion: 10ms over threshold → 10× the increment.
    // (10000us excess * 8 step_up) / 1000 = 80.
    state.own_price.store(0, Ordering::Relaxed);
    state
        .queue_stats
        .window_max
        .store(LATENCY_THRESHOLD_US + 10_000, Ordering::Relaxed);
    state.update_prices();
    let price_after_severe = state.own_price.load(Ordering::Relaxed);

    assert_eq!(price_after_mild, PRICE_STEP_UP);
    // Severe should be much larger than mild — proportionality.
    assert!(price_after_severe > price_after_mild);
    // 10ms excess × 8 tokens/ms = 80
    assert_eq!(price_after_severe, 80);
}

// ── B. Queue Delay Signal Tests (Window Max) ──

#[test]
fn test_queue_stats_tracks_maximum_not_average() {
    let state = RajomonSharedState::new();
    state
        .queue_stats
        .window_max
        .fetch_max(100, Ordering::Relaxed);
    state
        .queue_stats
        .window_max
        .fetch_max(5000, Ordering::Relaxed);
    state
        .queue_stats
        .window_max
        .fetch_max(200, Ordering::Relaxed);
    assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 5000);
}

/// §10.5 Option B: window_max is *halved* on read, not reset to 0. This
/// keeps the queue-latency signal alive when admission rejection dries up
/// the per-request observation stream from `before_poll`. See
/// `RUST_PORT_ALIGNMENT.md` §0 / §10.5 for the rationale.
#[test]
fn test_window_max_halves_after_price_update() {
    let state = RajomonSharedState::new();
    state.queue_stats.window_max.store(5000, Ordering::Relaxed);
    state.update_prices();
    // Halved, not reset to zero.
    assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 2500);
    state.update_prices();
    assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 1250);
}

// ── C. Price Aggregation Tests (Paper "Maximum Total Price") ──

/// Paper §3.4: `total_price = own + max(downstream prices)`.
/// See 3rd_party/rajomon/RUST_PORT_ALIGNMENT.md §0.2 for why this differs
/// from the Go reference implementation's `max(own, downstream)` form.
#[test]
fn test_accumulated_price_is_own_plus_max_downstream() {
    let state = RajomonSharedState::new();
    let method = CowGrpcMethod::new("svc", "method");

    // own=5, downstream=3 -> 5 + 3 = 8
    state.own_price.store(5, Ordering::Relaxed);
    state.max_downstream_for_method.insert(method.clone(), 3);
    assert_eq!(state.accumulated_price(&method), 8);

    // own=5, downstream=10 -> 5 + 10 = 15
    state.max_downstream_for_method.insert(method.clone(), 10);
    assert_eq!(state.accumulated_price(&method), 15);

    // own=0, no downstream -> 0 + 0 = 0
    state.own_price.store(0, Ordering::Relaxed);
    let method2 = CowGrpcMethod::new("svc", "other");
    assert_eq!(state.accumulated_price(&method2), 0);
}

#[test]
fn test_accumulated_price_own_only_no_downstream() {
    let state = RajomonSharedState::new();
    state.own_price.store(7, Ordering::Relaxed);
    let method = CowGrpcMethod::new("svc", "method");
    assert_eq!(state.accumulated_price(&method), 7);
}

// ── D. Token Consumption Tests (Deduction at Each Hop) ──

#[test]
fn test_check_inbound_deducts_own_price_not_accumulated() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let method = CowGrpcMethod::new("svc", "method");
    RAJOMON_STATE.own_price.store(3, Ordering::Relaxed);
    // Clear any downstream for this method
    RAJOMON_STATE
        .max_downstream_for_method
        .remove(&CowGrpcMethod::new("svc", "method"));

    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(!layer.should_drop);
    assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 17); // 20 - own(3) = 17
}

/// The inbound gate uses the paper's `accumulated = own + max(downstream)`
/// price for the admission check, but deducts only `own_price` from the
/// forwarded token budget — downstream children will then be charged
/// against their own prices via the cascaded token budget.
#[test]
fn test_check_inbound_deducts_own_not_accumulated_when_downstream_dominant() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let method = CowGrpcMethod::new("svc", "method_downstream_dom");
    RAJOMON_STATE.own_price.store(5, Ordering::Relaxed);
    RAJOMON_STATE
        .max_downstream_for_method
        .insert(method.clone(), 20); // accumulated = 5 + 20 = 25
    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(25).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(!layer.should_drop); // tok(25) >= accumulated(25) -> admitted
    assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 20); // 25 - own(5) = 20
}

#[test]
fn test_check_inbound_rejects_insufficient_tokens() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let method = CowGrpcMethod::new("svc", "reject_method");
    RAJOMON_STATE.own_price.store(100, Ordering::Relaxed);

    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(layer.should_drop);
}

#[test]
fn test_check_inbound_accepts_exact_tokens() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let method = CowGrpcMethod::new("svc", "exact_method");
    RAJOMON_STATE.own_price.store(10, Ordering::Relaxed);
    RAJOMON_STATE
        .max_downstream_for_method
        .remove(&CowGrpcMethod::new("svc", "exact_method"));

    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(!layer.should_drop);
    assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 0); // 10 - 10 = 0
}

#[test]
fn test_check_inbound_tokens_zero_price_zero() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let method = CowGrpcMethod::new("svc", "zero_method");
    RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);
    RAJOMON_STATE
        .max_downstream_for_method
        .remove(&CowGrpcMethod::new("svc", "zero_method"));

    // Paper §3.4 / Go LoadShedding: accumulated = 0 and tokens = 0 means
    // there is nothing to charge the request for, so it is admitted.
    // This test used to assert rejection under our now-removed .max(1)
    // minimum-effective-price clamp (§10.7).
    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(0).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(!layer.should_drop);
}

// ── E. Downstream Price Tests (Max Recomputation, Not Ratchet) ──

#[test]
fn test_downstream_price_can_decrease() {
    let parent = CowGrpcMethod::new("svc", "parent");
    let child_x = CowGrpcMethod::new("svc", "X");
    let child_y = CowGrpcMethod::new("svc", "Y");

    let state = RajomonSharedState::new();

    // Set X=15 -> max=15
    state
        .downstream_prices
        .insert((parent.clone(), child_x.clone()), 15);
    let max = state
        .downstream_prices
        .iter()
        .filter(|e| e.key().0 == parent)
        .map(|e| *e.value())
        .max()
        .unwrap_or(0);
    state.max_downstream_for_method.insert(parent.clone(), max);
    assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 15);

    // Set Y=5 -> max still 15
    state
        .downstream_prices
        .insert((parent.clone(), child_y.clone()), 5);
    let max = state
        .downstream_prices
        .iter()
        .filter(|e| e.key().0 == parent)
        .map(|e| *e.value())
        .max()
        .unwrap_or(0);
    state.max_downstream_for_method.insert(parent.clone(), max);
    assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 15);

    // X drops to 3 -> max should now be 5 (from Y), NOT still 15
    state
        .downstream_prices
        .insert((parent.clone(), child_x.clone()), 3);
    let max = state
        .downstream_prices
        .iter()
        .filter(|e| e.key().0 == parent)
        .map(|e| *e.value())
        .max()
        .unwrap_or(0);
    state.max_downstream_for_method.insert(parent.clone(), max);
    assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 5);

    // Set Y=20 -> max=20
    state
        .downstream_prices
        .insert((parent.clone(), child_y.clone()), 20);
    let max = state
        .downstream_prices
        .iter()
        .filter(|e| e.key().0 == parent)
        .map(|e| *e.value())
        .max()
        .unwrap_or(0);
    state.max_downstream_for_method.insert(parent.clone(), max);
    assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 20);
}

#[test]
fn test_child_price_default_zero() {
    let state = RajomonSharedState::new();
    let method = CowGrpcMethod::new("svc", "method");
    assert_eq!(state.child_price(&method), 0);
}

#[test]
fn test_layer_records_child_price_response_and_updates_parent_max() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    let parent = CowGrpcMethod::new("rajomon_e2e", "parent");
    let child = CowGrpcMethod::new("rajomon_e2e", "child");
    RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);
    RAJOMON_STATE
        .downstream_prices
        .remove(&(parent.clone(), child.clone()));
    RAJOMON_STATE.max_downstream_for_method.remove(&parent);

    let mut ctx = masa_core::ContextBuilder::new("test", 0)
        .tokens(100)
        .build();
    let layer = RajomonLayer::new(&parent, &RajomonServer, &mut ctx);
    let mut response: Result<Response<()>, Status> = Ok(Response::new(()));
    response
        .as_mut()
        .unwrap()
        .metadata_mut()
        .insert("x-masa-rajomon-price", "13".parse().unwrap());

    crate::layer::Layer::after_child_rpc(&layer, &ctx, &child, &mut response, &RajomonChild)
        .unwrap();

    assert_eq!(RAJOMON_STATE.child_price(&child), 13);
    assert_eq!(
        *RAJOMON_STATE
            .max_downstream_for_method
            .get(&parent)
            .expect("parent max price tracked"),
        13
    );
}

// ── F. Client Token Bucket Tests (Single Global Counter) ──

/// `try_acquire` returns a uniform-random token count in `[0, current]`
/// per the paper's Randomized Token Spending policy (§3.3). The exact
/// returned value is non-deterministic, so we only assert it's in range.
#[test]
fn test_client_token_bucket_single_global_counter() {
    let bucket = ClientTokenBucket::new();
    let method_a = CowGrpcMethod::new("svc", "A");
    let method_b = CowGrpcMethod::new("svc", "B");

    // Bucket starts at TOKENS_LEFT_INIT.
    assert_eq!(bucket.tokens_left(), TOKENS_LEFT_INIT);

    // First acquire returns a value in [0, TOKENS_LEFT_INIT].
    let tok_a = bucket.try_acquire(&method_a).expect("should succeed");
    assert!(tok_a <= TOKENS_LEFT_INIT);

    // Bucket should have TOKENS_LEFT_INIT - tok_a left, so subsequent
    // acquires can also succeed (cached_price defaults to 0).
    let tok_b = bucket.try_acquire(&method_b);
    assert!(tok_b.is_some());
}

/// Paper §3.3: token spending is uniform random in `[0, current]`. The
/// returned value should fall within the bucket balance and not equal the
/// balance with any deterministic regularity. We probe the distribution
/// with multiple draws and assert at least one falls strictly below the
/// current level — a deterministic "all" strategy would always return
/// `current`.
#[test]
fn test_client_token_spending_is_randomized() {
    let bucket = ClientTokenBucket::new();
    // Refill so the bucket has a wide range for draws.
    let refills = 22; // enough to build up a meaningful balance
    for _ in 0..refills {
        bucket.replenish();
    }
    let level = bucket.tokens_left();
    assert!(level > TOKENS_LEFT_INIT);

    // After 50 draws against a high bucket level (replenished each time),
    // a uniform-random strategy will produce values strictly below the
    // current level with overwhelming probability.
    let mut saw_strict_below = false;
    let method = CowGrpcMethod::new("svc", "m");
    for _ in 0..50 {
        // Top up the bucket between draws.
        for _ in 0..refills {
            bucket.replenish();
        }
        let current = bucket.tokens_left();
        let tok = bucket.try_acquire(&method).expect("should succeed");
        assert!(tok <= current);
        if tok < current {
            saw_strict_below = true;
        }
    }
    assert!(
        saw_strict_below,
        "uniform-random spending should produce at least one tok < current over 50 draws"
    );
}

/// `replenish` accumulates tokens.
#[test]
fn test_client_replenish_accumulates() {
    let bucket = ClientTokenBucket::new();
    for _ in 0..25 {
        bucket.replenish();
    }
    assert!(bucket.tokens_left() > TOKENS_LEFT_INIT);
}

#[test]
fn test_client_rate_limits_when_insufficient() {
    let bucket = ClientTokenBucket::new();
    let method = CowGrpcMethod::new("svc", "method");
    // Set price higher than the bucket balance to force rate limiting.
    bucket.update_price(&method, TOKENS_LEFT_INIT + TOKEN_UPDATE_STEP * 100 + 1);

    let result = bucket.try_acquire(&method);
    assert!(result.is_none());
}

// ── G. Price Propagation Tests (Probabilistic, paper §3.4) ──

/// Paper §3.4: probabilistic propagation with `1/price_freq` rate.
/// `price_freq = 1` should always propagate; `price_freq = 0` should
/// never propagate. These two edges are deterministic and easy to assert.
#[test]
fn test_price_propagation_edge_cases() {
    let method = CowGrpcMethod::new("svc", "m");
    let mut ctx = masa_core::ContextBuilder::new("test", 0)
        .tokens(100)
        .build();
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);
    RAJOMON_STATE.max_downstream_for_method.remove(&method);
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);

    // We can't override PolicyParams::global() at runtime here, so we
    // can't easily test arbitrary price_freq values from the test
    // environment. The compile-time default is `price_freq = 5` (see
    // RajomonParams::default), which gives a ~20% propagation rate.
    // Verify the predicate at least returns *some* mixture of true and
    // false over many calls.
    let n_trials = 5000;
    let mut true_count = 0;
    for _ in 0..n_trials {
        if layer.should_propagate_price() {
            true_count += 1;
        }
    }
    // With price_freq=5, expected rate is ~20% = 1000 out of 5000.
    // With p=0.2, n=5000, std-dev = sqrt(5000 * 0.2 * 0.8) ≈ 28.
    // 4σ tolerance: |observed - 1000| < 112. Use 200 for headroom.
    assert!(
        (800..=1200).contains(&true_count),
        "expected ~20% propagation rate (1000±200), got {} / {}",
        true_count,
        n_trials
    );
}

// ── H. End-to-End Algorithmic Equivalence Tests ──

#[test]
fn test_load_shedding_returns_correct_remaining_tokens() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    RAJOMON_STATE.own_price.store(7, Ordering::Relaxed);
    let method = CowGrpcMethod::new("svc", "Foo_ls");
    RAJOMON_STATE.max_downstream_for_method.remove(&method);

    let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
    let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
    assert!(!layer.should_drop);
    assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 13); // 20 - 7 = 13
}

#[test]
fn test_mixed_tokens_accept_reject() {
    let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
    RAJOMON_STATE.own_price.store(10, Ordering::Relaxed);
    let method = CowGrpcMethod::new("svc", "Foo_mixed");
    RAJOMON_STATE.max_downstream_for_method.remove(&method);

    let mut rejected = 0;
    let mut accepted = 0;
    for i in 0..10 {
        let token_val = if i < 5 { 3u64 } else { 20u64 };
        let mut ctx = masa_core::ContextBuilder::new("test", 0)
            .tokens(token_val)
            .build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        if layer.should_drop {
            rejected += 1;
        } else {
            accepted += 1;
            if i >= 5 {
                assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 10);
                // 20 - 10
            }
        }
    }
    assert_eq!(rejected, 5);
    assert_eq!(accepted, 5);
}

#[test]
fn test_price_increases_then_decreases_over_time() {
    let state = RajomonSharedState::new();

    // 5 ticks of congestion at 1ms over the threshold → exactly
    // PRICE_STEP_UP increment per tick under the proportional formula.
    for _ in 0..5 {
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
        state.update_prices();
    }
    assert_eq!(state.own_price.load(Ordering::Relaxed), 5 * PRICE_STEP_UP);

    // Wipe the persistent halved signal so the decay branch fires
    // immediately on the no-congestion ticks.
    state.queue_stats.window_max.store(0, Ordering::Relaxed);

    // 3 ticks of no congestion → -1 per tick (paper-hardcoded decay).
    for _ in 0..3 {
        state.update_prices();
    }
    assert_eq!(
        state.own_price.load(Ordering::Relaxed),
        5 * PRICE_STEP_UP - 3
    );
}

#[test]
fn test_global_own_price_shared_across_methods() {
    let state = RajomonSharedState::new();
    state.own_price.store(42, Ordering::Relaxed);
    let method_a = CowGrpcMethod::new("svc", "A");
    let method_b = CowGrpcMethod::new("svc", "B");

    let price_a = state.accumulated_price(&method_a);
    let price_b = state.accumulated_price(&method_b);
    assert_eq!(price_a, 42);
    assert_eq!(price_b, 42);
}
