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
/// `token_update_step / token_update_rate_ms` tokens/ms, capped at
/// `max_token`. Each outgoing request spends a *uniform random* amount
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

    /// **Deprecated under paper-aligned `update_prices`.**
    /// Paper §3.4 hardcodes the decay step at -1 token per tick when
    /// queueing falls below half the threshold. This field is retained for
    /// JSON-schema backward compatibility but is **ignored** by the current
    /// `update_prices` implementation.
    pub price_step_down: u64,

    /// Maximum server price (a Rust safety extension; not in the paper).
    /// The paper has no upper bound on price — prices are self-regulating
    /// via the feedback loop. Default is `u64::MAX` (effectively unlimited)
    /// to match paper fidelity. See `RUST_PORT_ALIGNMENT.md` §10 item C.
    pub price_cap: u64,

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
    /// startup. The bucket then replenishes asynchronously up to `max_token`.
    pub tokens_left_init: u64,

    /// Mean inter-replenishment interval (in milliseconds) for the
    /// client-side token bucket. Paper §3.3 specifies a Poisson process,
    /// which our implementation realizes via an exponential distribution
    /// with rate `1 / token_update_rate_ms`.
    pub token_update_rate_ms: u64,

    /// Tokens added on each client-bucket replenishment event.
    pub token_update_step: u64,

    /// Cap on the client-side token bucket. The bucket saturates at
    /// `max_token` regardless of how many replenishments fire. Each
    /// outgoing request then spends a uniform random amount in
    /// `[0, current_balance]` (paper §3.3 Randomized Token Spending).
    pub max_token: u64,
}

impl Default for RajomonParams {
    fn default() -> Self {
        Self {
            price_update_rate_ms: 10,
            latency_threshold_us: 5_000,
            price_step_up: 8,
            price_step_down: 2,
            price_cap: u64::MAX,
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
///
/// `reject_prob = (virt_er_rate * reject_scale + goodput_divergence * goodput_divergence_weight).min(1.0)`
///
/// The ER rate EMA tracks the fraction of admitted requests that end in an
/// early return; the exponential decay provides natural phase reset when idle.
/// The goodput divergence term fires when throughput is falling (load spike onset)
/// and vanishes at steady state, acting as a derivative (D) term.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PredParams {
    /// Time constant (seconds) for the early-return rate EMA.
    ///
    /// Controls both the update speed of `er_rate` in `record_outcome` (via
    /// time-corrected alpha) and how quickly it decays in `should_admit` when
    /// no completions arrive (natural phase reset without explicit resets).
    /// Default 2.0s gives a consistent ~2s response time regardless of RPS.
    pub tau_er: f64,
    /// Fast EMA time constant (seconds) for goodput rate.
    pub tau_fast: f64,
    /// Slow EMA time constant (seconds) for goodput rate.
    pub tau_slow: f64,
    /// Previously used to cap rejection probability in the "healthy" goodput
    /// state. No longer used in the admission decision — kept for backwards
    /// compatibility with existing config files.
    pub max_reject_floor: f64,
    /// Rejection scale factor on `virt_er_rate`. Default 1.0.
    ///
    /// **Do not set above 1.0.** Values > 1.0 increase the closed-loop gain above
    /// 1.0 (loop_gain = reject_scale × C×R/A*²), causing oscillation. Use
    /// `goodput_divergence_weight` instead for tighter admission at high load.
    pub reject_scale: f64,
    /// Weight for the goodput-divergence derivative term.
    ///
    /// At each admission check:
    ///   `D = max(0.0, (goodput_slow − goodput_fast) / goodput_slow)`
    ///   `reject_prob = (virt_er_rate * reject_scale + D * goodput_divergence_weight).min(1.0)`
    ///
    /// `D` is positive when goodput is *falling* (fast EMA below slow EMA), which
    /// happens within ~`tau_fast` seconds of a load spike — well before ER rate
    /// builds up over ~`tau_er` seconds. At steady state `goodput_fast ≈ goodput_slow`
    /// so `D → 0` and steady-state equilibrium and loop gain are unchanged.
    ///
    /// Default 0.0 (disabled, backward compatible). Start with 0.5.
    pub goodput_divergence_weight: f64,
    /// Variance multiplier for the LatencyMeanVar estimator.
    /// estimate = mean + k * stddev. 0.0 = pure mean estimator (default).
    pub estimator_k: f64,
    /// AIMD additive increase per healthy observation window.
    ///
    /// When the window's ER fraction is below `aimd_er_threshold`, the admission
    /// probability is increased by this amount: `admit_p = min(1.0, admit_p + alpha)`.
    /// Default 0.0 disables AIMD entirely (backward compatible).
    /// Suggested starting value: 0.05 (recover from 0 → 1 in ~20 healthy windows = 200ms).
    pub aimd_alpha: f64,
    /// AIMD multiplicative decrease factor applied on an overloaded window.
    ///
    /// When the window's ER fraction exceeds `aimd_er_threshold`:
    /// `admit_p = admit_p * beta`.
    /// Must be in (0.0, 1.0). Default 0.875 cuts admission by 12.5% per overloaded window,
    /// matching classic TCP-style response.
    pub aimd_beta: f64,
    /// ER-fraction threshold above which a window is considered overloaded.
    ///
    /// Each 10 ms observation window votes: if `er_count / window_total > threshold`,
    /// AIMD applies a multiplicative decrease; otherwise an additive increase.
    /// Default 0.10 (10% ER rate triggers decrease).
    pub aimd_er_threshold: f64,
}

impl Default for PredParams {
    fn default() -> Self {
        Self {
            tau_er: 2.0,
            tau_fast: 0.5,
            tau_slow: 5.0,
            max_reject_floor: 0.10,
            reject_scale: 1.0,
            goodput_divergence_weight: 0.0,
            estimator_k: 0.0,
            aimd_alpha: 0.0,
            aimd_beta: 0.875,
            aimd_er_threshold: 0.10,
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
        assert_eq!(p.rajomon.max_token, 100);
        assert_eq!(p.rajomon.price_cap, u64::MAX);
        assert!(p.pred.tau_er > 0.0);
        assert!(p.pred.tau_fast < p.pred.tau_slow);
        assert_eq!(p.pred.reject_scale, 1.0);
        assert_eq!(p.pred.goodput_divergence_weight, 0.0);
        assert_eq!(p.pred.aimd_alpha, 0.0);
    }

    #[test]
    fn test_partial_json_uses_defaults() {
        let json = r#"{"rajomon": {"max_token": 200}}"#;
        let p: PolicyParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.rajomon.max_token, 200);
        assert_eq!(p.rajomon.price_update_rate_ms, 10);
        assert_eq!(p.pred.tau_er, 2.0);
    }

    #[test]
    fn test_empty_json_uses_all_defaults() {
        let p: PolicyParams = serde_json::from_str("{}").unwrap();
        let d = PolicyParams::default();
        assert_eq!(p.rajomon.max_token, d.rajomon.max_token);
        assert_eq!(p.pred.tau_er, d.pred.tau_er);
    }
}
