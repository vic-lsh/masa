// Runtime-configurable policy parameters.
//
// Parameters for scheduling policies (ac_rajomon, ac_pred) that previously
// required recompilation are now loaded at startup from a JSON file.
//
// The file path is read from the `MASA_POLICY_PARAMS_PATH` environment
// variable. If the variable is unset or the file cannot be opened, all
// parameters fall back to their built-in defaults.
//
// In the experiment runner, `policy_param.json` lives in each experiment's
// input directory (`exp/{app}/in/{experiment}/policy_param.json`). The
// runner writes it to the per-run output directory and mounts it into every
// service container as `/usr/policy_params.json`.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Tunable parameters for the Rajomon token-based admission control policy.
///
/// All defaults reproduce the behaviour of the original Go implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RajomonParams {
    /// Server-side price tick interval in milliseconds.
    pub price_update_rate_ms: u64,
    /// Queue-latency threshold (µs) above which the server raises its price.
    pub latency_threshold_us: u64,
    /// Additive price increment per tick when congested.
    pub price_step_up: u64,
    /// Additive price decrement per tick when not congested.
    pub price_step_down: u64,
    /// Maximum server price. Should be ≤ max_token to allow some admission.
    pub price_cap: u64,
    /// Initial server price on startup.
    pub init_price: u64,
    /// Deterministic price-propagation frequency: send price every 1/N requests.
    pub price_freq: u64,
    /// Initial client token-bucket balance.
    pub tokens_left_init: u64,
    /// Client token-bucket replenishment interval in milliseconds.
    pub token_update_rate_ms: u64,
    /// Tokens added per replenishment tick.
    pub token_update_step: u64,
    /// Maximum token value. Loadgen draws uniform random bids in [0, max_token].
    /// price_cap should be ≤ max_token.
    pub max_token: u64,
}

impl Default for RajomonParams {
    fn default() -> Self {
        Self {
            price_update_rate_ms: 10,
            latency_threshold_us: 5_000,
            price_step_up: 8,
            price_step_down: 2,
            price_cap: 60,
            init_price: 0,
            price_freq: 5,
            tokens_left_init: 10,
            token_update_rate_ms: 10,
            token_update_step: 5,
            max_token: 100,
        }
    }
}

/// Tunable parameters for the predictive admission control policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PredParams {
    /// Age (seconds) after which a bottleneck utilization reading is considered stale.
    pub staleness_secs: f64,
    /// Utilization assumed when no (or stale) bottleneck data is available.
    pub staleness_default: f32,
    /// Target bottleneck utilization fraction for the token-bucket rate controller.
    pub util_target: f64,
    /// Multiplicative rate-adjustment factor per second.
    pub adjust_rate: f64,
    /// Maximum burst window in seconds (token-bucket capacity = initial_budget_rate × max_burst_secs).
    pub max_burst_secs: f64,
    /// Initial token-bucket refill rate in µs of compute budget per second.
    pub initial_budget_rate: f64,
    /// Probabilistic smoothing exponent (1.0=linear, high=binary, 0.0=disabled).
    pub prob_smooth: f64,
    /// ER fraction at which pseudo-utilization saturates to 1.0.
    pub er_threshold: f64,
    /// Slow EMA alpha for ER tracker (averages over oscillation cycles).
    pub er_alpha: f64,
    /// Variance multiplier for the LatencyMeanVar estimator.
    /// estimate = mean + k * stddev. 0.0 = pure mean estimator (default).
    pub estimator_k: f64,
}

impl Default for PredParams {
    fn default() -> Self {
        Self {
            staleness_secs: 2.0,
            staleness_default: 0.5,
            util_target: 0.80,
            adjust_rate: 2.0,
            max_burst_secs: 0.005,
            initial_budget_rate: 5_000_000.0,
            prob_smooth: 1.0,
            er_threshold: 0.2,
            er_alpha: 0.05,
            estimator_k: 0.0,
        }
    }
}

/// Combined policy parameters loaded once at startup.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PolicyParams {
    pub rajomon: RajomonParams,
    pub pred: PredParams,
}

static POLICY_PARAMS: OnceLock<PolicyParams> = OnceLock::new();

impl PolicyParams {
    /// Return the global policy parameters.
    ///
    /// On the first call, attempts to load from the path stored in
    /// `MASA_POLICY_PARAMS_PATH`. Falls back to built-in defaults if the
    /// variable is unset or the file cannot be read / parsed.
    pub fn global() -> &'static PolicyParams {
        POLICY_PARAMS.get_or_init(|| {
            if let Ok(path) = std::env::var("MASA_POLICY_PARAMS_PATH") {
                match std::fs::File::open(&path) {
                    Ok(file) => match serde_json::from_reader(std::io::BufReader::new(file)) {
                        Ok(params) => {
                            log::info!("Loaded policy params from {}", path);
                            return params;
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to parse policy params from {}: {}, using defaults",
                                path,
                                e
                            );
                        }
                    },
                    Err(e) => {
                        log::warn!(
                            "Failed to open policy params file {}: {}, using defaults",
                            path,
                            e
                        );
                    }
                }
            }
            PolicyParams::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_are_sane() {
        let p = PolicyParams::default();
        assert_eq!(p.rajomon.max_token, 100);
        assert!(p.rajomon.price_cap <= p.rajomon.max_token);
        assert_eq!(p.pred.util_target, 0.80);
    }

    #[test]
    fn test_partial_json_uses_defaults() {
        let json = r#"{"rajomon": {"max_token": 200}}"#;
        let p: PolicyParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.rajomon.max_token, 200);
        // Other rajomon fields should be defaults
        assert_eq!(p.rajomon.price_update_rate_ms, 10);
        // pred fields should be defaults
        assert_eq!(p.pred.util_target, 0.80);
    }

    #[test]
    fn test_empty_json_uses_all_defaults() {
        let p: PolicyParams = serde_json::from_str("{}").unwrap();
        let d = PolicyParams::default();
        assert_eq!(p.rajomon.max_token, d.rajomon.max_token);
        assert_eq!(p.pred.util_target, d.pred.util_target);
    }
}
