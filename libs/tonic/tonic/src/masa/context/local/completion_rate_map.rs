use std::collections::HashMap;
use std::sync::Mutex;

/// Minimum admission probability — ensures we always probe a fraction of requests
/// even when the empirical completion rate has dropped to 0.
pub(crate) const PROBE_FLOOR: f64 = 0.05;

const ALPHA_FALL: f64 = 0.1; // fast deterioration response
const ALPHA_RISE: f64 = 0.05; // slow recovery

/// Maps (api, time_left_bucket) → empirical completion probability ∈ [0.0, 1.0].
///
/// Initialized to 1.0 on first access. Updated via asymmetric EMA each time a
/// request admitted through the empirical admission check completes or fails.
#[derive(Debug)]
pub(crate) struct CompletionRateMap {
    map: Mutex<HashMap<(String, usize), f64>>,
}

impl CompletionRateMap {
    pub(crate) fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Returns stored completion probability for (api, bucket), or 1.0 if unseen.
    pub(crate) fn get_p(&self, api: &str, bucket: usize) -> f64 {
        let map = self.map.lock().unwrap();
        *map.get(&(api.to_string(), bucket)).unwrap_or(&1.0)
    }

    /// EMA update: asymmetric alpha depending on whether outcome improved or worsened.
    pub(crate) fn update(&self, api: &str, bucket: usize, completed: bool) {
        let outcome = if completed { 1.0 } else { 0.0 };
        let mut map = self.map.lock().unwrap();
        let p = map.entry((api.to_string(), bucket)).or_insert(1.0);
        let alpha = if outcome < *p { ALPHA_FALL } else { ALPHA_RISE };
        *p += alpha * (outcome - *p);
        *p = p.clamp(0.0, 1.0);
    }
}

/// Converts time_left (microseconds) to a SLO-relative bucket index.
///
/// Bucket boundaries (as fraction of SLO):
/// - 0: < 0.10
/// - 1: [0.10, 0.20)
/// - 2: [0.20, 0.40)
/// - 3: [0.40, 0.70)
/// - 4: [0.70, 1.00)
/// - 5: ≥ 1.00 (full budget remaining, never shed)
pub(crate) fn time_left_to_bucket(time_left_us: u64, slo_us: u64) -> usize {
    if slo_us == 0 {
        return 5;
    }
    // Use fixed-point arithmetic to avoid floating point division
    // frac_10000 = (time_left * 10000) / slo, representing frac * 10000
    let frac_10000 = time_left_us.saturating_mul(10000) / slo_us;
    if frac_10000 < 1000 {
        0
    } else if frac_10000 < 2000 {
        1
    } else if frac_10000 < 4000 {
        2
    } else if frac_10000 < 7000 {
        3
    } else if frac_10000 < 10000 {
        4
    } else {
        5
    }
}

/// Deterministic pseudo-random value in [0, 1) derived from request_id and a salt.
///
/// Uses a multiplicative hash to produce per-hop variation without a thread-local RNG.
pub(crate) fn deterministic_rand(request_id: u64, salt: u64) -> f64 {
    let mixed = request_id
        .wrapping_add(salt)
        .wrapping_mul(6364136223846793005);
    // Extract 53 bits of mantissa for uniform [0, 1) float
    (mixed >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_left_to_bucket() {
        let slo = 100_000; // 100ms in us
        assert_eq!(time_left_to_bucket(0, slo), 0);
        assert_eq!(time_left_to_bucket(5_000, slo), 0); // 5% < 10%
        assert_eq!(time_left_to_bucket(10_000, slo), 1); // 10%
        assert_eq!(time_left_to_bucket(15_000, slo), 1); // 15%
        assert_eq!(time_left_to_bucket(20_000, slo), 2); // 20%
        assert_eq!(time_left_to_bucket(39_999, slo), 2); // just under 40%
        assert_eq!(time_left_to_bucket(40_000, slo), 3); // 40%
        assert_eq!(time_left_to_bucket(69_999, slo), 3); // just under 70%
        assert_eq!(time_left_to_bucket(70_000, slo), 4); // 70%
        assert_eq!(time_left_to_bucket(99_999, slo), 4); // just under 100%
        assert_eq!(time_left_to_bucket(100_000, slo), 5); // exactly 100%
        assert_eq!(time_left_to_bucket(200_000, slo), 5); // over 100%
    }

    #[test]
    fn test_time_left_to_bucket_zero_slo() {
        assert_eq!(time_left_to_bucket(100, 0), 5);
    }

    #[test]
    fn test_deterministic_rand_range() {
        for i in 0..1000u64 {
            let v = deterministic_rand(i, 42);
            assert!(v >= 0.0 && v < 1.0, "out of range: {}", v);
        }
    }

    #[test]
    fn test_completion_rate_map_default_p() {
        let map = CompletionRateMap::new();
        assert_eq!(map.get_p("Search", 2), 1.0);
    }

    #[test]
    fn test_completion_rate_map_update_fall() {
        let map = CompletionRateMap::new();
        // First failure: p goes from 1.0 down by ALPHA_FALL
        map.update("Search", 2, false);
        let p = map.get_p("Search", 2);
        let expected = 1.0 + ALPHA_FALL * (0.0 - 1.0);
        assert!((p - expected).abs() < 1e-10, "p={} expected={}", p, expected);
    }

    #[test]
    fn test_completion_rate_map_clamp() {
        let map = CompletionRateMap::new();
        // Drive p to near 0
        for _ in 0..100 {
            map.update("Search", 0, false);
        }
        let p = map.get_p("Search", 0);
        assert!(p >= 0.0 && p <= 1.0);
        assert!(p < 0.01);
    }
}
