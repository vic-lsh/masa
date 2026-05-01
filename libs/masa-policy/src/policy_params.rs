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
/// Implements the algorithm described in the NSDI '25 paper *"Rajomon:
/// Decentralized and Coordinated Overload Control for Latency-Sensitive
/// Microservices"* (Xing et al.). For an exhaustive comparison against the
/// paper and the upstream Go reference (`3rd_party/rajomon/`), see
/// `3rd_party/rajomon/RUST_PORT_ALIGNMENT.md`.
///
/// **Algorithm summary** (paper §3.4 "Proportional Price Updates"):
/// At each tick (`price_update_rate_ms`), the server reads the maximum
/// queueing delay observed in the previous window. If it exceeds the
/// threshold, the price is *increased proportionally* to the excess
/// (`(excess_us * price_step_up) / 1000`, paper says ~3-13 tokens per 1ms
/// excess is typical). If queueing falls below half the threshold, the
/// price is *decreased by 1* (hardcoded in the paper). Otherwise, the
/// price is held (hysteresis dead band `[threshold/2, threshold]`).
///
/// **Total price** (paper §3.4 "Maximum Total Price"):
/// `total_price(service) = own_price + max(downstream_total_prices)`.
/// A request with `tokens` is admitted iff `tokens >= total_price`.
///
/// **Lazy price propagation** (paper §3.4):
/// Each response attaches the local price with probability `1/price_freq`
/// (probabilistic per-call Bernoulli draw, not a deterministic modulo).
///
/// **Client-side token bucket** (paper §3.3):
/// Tokens replenish via a Poisson process at rate
/// `token_update_step / token_update_rate_ms` tokens/ms.
/// Each outgoing request spends a *uniform random* amount
/// from `[0, current_balance]` (paper §3.3 "Randomized Token Spending");
/// deterministic "all-in" spending is explicitly called out by the paper
/// as making AQM ineffective.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RajomonParams {
    /// Server-side price-update tick interval in milliseconds.
    /// Paper §3.4 suggests this typically ranges from 1ms to 20ms.
    pub price_update_rate_ms: u64,

    /// Queueing-delay threshold in microseconds.
    /// The price climbs when observed queue latency exceeds this value
    /// and decays when it drops below half this value (`threshold / 2`).
    /// Paper §3.4 suggests this typically ranges from 1ms to 20ms.
    pub latency_threshold_us: u64,

    /// Proportional price-increase coefficient: tokens added per 1ms of
    /// queueing-delay excess over `latency_threshold_us`.
    /// The full increment per tick is
    /// `((excess_us * price_step_up) / 1000).max(1)`.
    /// Paper §3.4: typical range is 3-13. Bumping this above the paper
    /// range makes the controller more aggressive on the climb side.
    pub price_step_up: u64,

    /// Initial server price on worker startup.
    /// Default 0 matches the Go reference. The paper doesn't specify.
    pub init_price: u64,

    /// Inverse propagation probability: each response attaches the price
    /// with probability `1 / price_freq` via a per-call Bernoulli draw.
    /// Paper §3.4 example: 20% propagation rate, i.e. `price_freq = 5`.
    /// `price_freq = 1` means always propagate. `price_freq = 0` disables
    /// propagation entirely.
    pub price_freq: u64,

    /// Initial value of the client-side `CLIENT_TOKEN_BUCKET` on process
    /// startup.
    pub tokens_left_init: u64,

    /// Mean inter-replenishment interval (in milliseconds) for the
    /// client-side token bucket. Paper §3.3 specifies a Poisson process,
    /// which our implementation realizes via an exponential distribution
    /// with rate `1 / token_update_rate_ms`.
    pub token_update_rate_ms: u64,

    /// Tokens added on each client-bucket replenishment event.
    pub token_update_step: u64,
}

impl Default for RajomonParams {
    fn default() -> Self {
        Self {
            price_update_rate_ms: 10,
            latency_threshold_us: 5_000,
            price_step_up: 8,
            init_price: 0,
            price_freq: 5,
            tokens_left_init: 10,
            token_update_rate_ms: 10,
            token_update_step: 5,
        }
    }
}

/// Tunable parameters for the predictive admission control policy.
///
/// `reject_prob = (1 - admit_p) * exp(-idle_elapsed / tau_er)`
///
/// `admit_p` is updated per 50 ms window: multiplicative decrease (`admit_p *= beta`)
/// when the window's ER fraction exceeds `aimd_er_threshold`; additive increase
/// (`admit_p += alpha`) otherwise. The exponential term provides natural phase reset
/// when traffic is sparse — idle_elapsed grows, reject_prob decays to 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PredParams {
    /// Time constant (seconds) for idle decay in `should_admit`.
    ///
    /// As time passes since the last window close, reject_prob decays:
    /// `reject_prob = (1 - admit_p) * exp(-idle_elapsed / tau_er)`.
    /// Larger values keep admission restricted longer after an overload episode.
    /// Default 2.0 s.
    pub tau_er: f64,
    /// Variance multiplier for the LatencyMeanVar estimator used by abort_slack.
    /// `estimate = mean + k * stddev`. 0.0 = pure mean estimator (default).
    pub estimator_k: f64,
    /// AIMD proportional additive increase per healthy 50 ms window.
    ///
    /// Actual increment is `alpha * (1 - admit_p)`, so recovery slows as
    /// admit_p approaches 1.0.  At admit_p = 0 the step equals alpha.
    /// Default 0.05.
    pub aimd_alpha: f64,
    /// AIMD base multiplicative decrease factor (severity-scaled).
    ///
    /// Effective factor is `beta.powf(er_sample / aimd_er_threshold)`:
    /// at the threshold boundary the cut equals `beta`; at 4× threshold
    /// the cut is `beta^4`.  Must be in (0.0, 1.0).  Default 0.875.
    pub aimd_beta: f64,
    /// ER-fraction threshold above which a 50 ms window is considered overloaded.
    ///
    /// Should be set just below the natural ER fraction at saturation
    /// (e.g. 0.40 if saturation produces ~41% ER fraction).
    /// Default 0.10.
    pub aimd_er_threshold: f64,
    /// Blend factor for fanout-corrected hard-deadline remaining-work estimates.
    ///
    /// `0.0` uses the fanout estimate directly when it is below the legacy
    /// per-edge estimate. `1.0` keeps the legacy per-edge hard-deadline
    /// estimate while still allowing fanout tracking and lookup. Intermediate
    /// values partially remove sibling-wait noise without fully loosening child
    /// deadlines. Soft scheduling priority continues to use the legacy full
    /// estimate.
    pub fanout_deadline_legacy_fraction: f64,
    /// Restrict fanout-aware after-child estimation to root-level parents.
    ///
    /// This is useful for workloads where internal fanouts are mostly
    /// conditional single-child steps: root fanout correction can still remove
    /// sibling-wait noise, while internal services keep the legacy low-overhead
    /// per-edge estimator.
    pub fanout_root_only: bool,
}

impl Default for PredParams {
    fn default() -> Self {
        Self {
            tau_er: 2.0,
            estimator_k: 0.0,
            aimd_alpha: 0.05,
            aimd_beta: 0.875,
            aimd_er_threshold: 0.10,
            fanout_deadline_legacy_fraction: 0.0,
            fanout_root_only: false,
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
                            log::info!(
                                "Policy params: {}",
                                serde_json::to_string(&params).unwrap()
                            );
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
            let params = PolicyParams::default();
            log::info!(
                "Using default policy params: {}",
                serde_json::to_string(&params).unwrap()
            );
            params
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_are_sane() {
        let p = PolicyParams::default();
        assert_eq!(p.rajomon.token_update_step, 5);
        assert!(p.pred.tau_er > 0.0);
        assert!(p.pred.aimd_alpha > 0.0);
        assert!(p.pred.aimd_beta > 0.0 && p.pred.aimd_beta < 1.0);
        assert!(p.pred.aimd_er_threshold > 0.0 && p.pred.aimd_er_threshold < 1.0);
    }

    #[test]
    fn test_partial_json_uses_defaults() {
        let json = r#"{"rajomon": {"token_update_step": 200}}"#;
        let p: PolicyParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.rajomon.token_update_step, 200);
        assert_eq!(p.rajomon.price_update_rate_ms, 10);
        assert_eq!(p.pred.tau_er, 2.0);
    }

    #[test]
    fn test_empty_json_uses_all_defaults() {
        let p: PolicyParams = serde_json::from_str("{}").unwrap();
        let d = PolicyParams::default();
        assert_eq!(p.rajomon.token_update_step, d.rajomon.token_update_step);
        assert_eq!(p.pred.tau_er, d.pred.tau_er);
    }
}
