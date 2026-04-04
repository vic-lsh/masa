// Predictive admission control layer — goodput-tracking token-bucket AC.
//
// When `ac_pred` is enabled, this layer runs compute-capacity admission
// control via a goodput-tracking token bucket. It sits after the
// estimation layer which handles latency tracking, deadline tightening,
// and feasibility checks independently.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use masa_core::Context;
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use crate::layer::est::estimator::DefaultLatencyEstimator;
use crate::layer::est::latency_map::{MethodKey, ParentToChildKey};
use crate::layer::est::state::LatencyEstimators;
use crate::policy_params::PolicyParams;
use crate::registry::MethodId;

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level predictive admission state (shared across requests).
#[derive(Debug)]
pub(crate) struct PredAdmissionServer {
    pred_admission: Arc<PredictiveAdmission>,
}

impl LayerServer for PredAdmissionServer {
    fn new() -> Self {
        Self {
            pred_admission: Arc::new(PredictiveAdmission::new()),
        }
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request predictive admission layer state.
///
/// Accesses the estimation layer's shared `LatencyEstimators` (via the
/// server-level `EstimationServer`) for cost estimates used by the
/// goodput-tracking admission controller.
#[derive(Debug)]
pub(crate) struct PredAdmissionLayer {
    pred_admission: Arc<PredictiveAdmission>,
    /// Reference to the shared estimators for cost lookups.
    est: LatencyEstimators<DefaultLatencyEstimator>,
    root_method_id: Option<MethodId>,
    rpc: CowGrpcMethod,
}

impl Layer for PredAdmissionLayer {
    type Server = PredAdmissionServer;
    type Child = PredAdmissionChild;

    fn new(method: &CowGrpcMethod, server: &PredAdmissionServer, ctx: &mut Context) -> Self {
        let root_method_id = ctx.root_method().map(|rm| {
            crate::MethodRegistry::global()
                .get_or_register(CowGrpcMethod::new(rm.service.clone(), rm.method.clone()))
        });
        Self {
            pred_admission: server.pred_admission.clone(),
            est: crate::layer::estimation::global_estimators(),
            root_method_id,
            rpc: method.clone(),
        }
    }

    /// Two-layer admission check before child RPC.
    ///
    /// - Layer 1 (every hop): floor-based deadline feasibility — reject if
    ///   estimated remaining wall-clock time exceeds deadline.
    /// - Layer 2 (ingress only, hop_count==0): compute-capacity admission
    ///   via goodput-tracking token bucket.
    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        _child_ctx: &mut PredAdmissionChild,
        _request: &mut tonic_core::Request<T>,
        _child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        use masa_core::time_now;

        let child_id = crate::MethodRegistry::global().get_or_register(child_method_name.clone());
        let key = ParentToChildKey::parent_rpc_method(
            crate::MethodRegistry::global().get_or_register(self.rpc.clone()),
        )
        .child_rpc_method(child_id);

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        // Layer 1: floor-based deadline feasibility
        let remaining = self.est.est_after_child_wallclock(key, time_left);
        let est_child = self.est.est_child_wallclock(key).unwrap_or(0);
        if time_now() + est_child + remaining.floor > ctx.e2e_deadline() {
            return Err(Status::new(
                Code::DeadlineExceeded,
                format!(
                    "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason=BeforeChildFeasibility",
                    self.rpc.service(),
                    self.rpc.method(),
                    child_method_name.service(),
                    child_method_name.method(),
                ),
            ));
        }

        // Layer 2: compute-capacity admission (ingress only)
        if ctx.hop_count() == 0 {
            let est_cost = if let Some(root_mid) = self.root_method_id {
                self.est.est_subtree_compute(MethodKey(root_mid)).unwrap_or(0)
            } else {
                self.est.est_child_wallclock(key).unwrap_or(0)
            };
            if !self.pred_admission.should_admit(est_cost) {
                return Err(Status::new(
                    Code::DeadlineExceeded,
                    format!(
                        "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason=TokenBucketRej",
                        self.rpc.service(),
                        self.rpc.method(),
                        child_method_name.service(),
                        child_method_name.method(),
                    ),
                ));
            }
        }

        Ok(())
    }

    /// Record goodput on successful child RPC completion (ingress only).
    #[inline]
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: &PredAdmissionChild,
    ) -> Result<(), Status> {
        use crate::context_ext::MasaResponseExt;

        if ctx.hop_count() == 0 {
            if let Ok(resp) = response {
                if let Some(child_ctx_resp) = resp.get_masa_context() {
                    if let Some(meta) = child_ctx_resp.response_meta() {
                        self.pred_admission.record_completion(meta.accumulated_compute_us);
                        if let Some(root_mid) = self.root_method_id {
                            self.est
                                .track_subtree_compute(MethodKey(root_mid), meta.accumulated_compute_us);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct PredAdmissionChild;

impl LayerChild for PredAdmissionChild {
    fn new() -> Self {
        Self
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Goodput-tracking admission controller
// ══════════════════════════════════════════════════════════════════════════

struct BudgetState {
    /// EMA of cost-weighted goodput (µs/s).
    goodput_rate: f64,
    /// Token bucket balance (µs).
    budget_us: f64,
    /// Timestamp of last admission check.
    last_update: Instant,
    /// EMA of per-decision rejection rate (0.0 = no rejections, 1.0 = all rejected).
    rejection_ema: f64,
}

impl std::fmt::Debug for BudgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetState")
            .field("goodput_rate", &self.goodput_rate)
            .field("budget_us", &self.budget_us)
            .finish()
    }
}

/// Goodput-tracking admission controller.
///
/// Sets the token-bucket refill rate to slightly above observed goodput
/// (successful completion throughput in µs/s).
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    state: Mutex<BudgetState>,
    /// Lock-free accumulator for completed child RPC costs (µs).
    completed_cost_us: AtomicU64,
}

impl PredictiveAdmission {
    pub(crate) fn new() -> Self {
        let p = &PolicyParams::global().pred;
        Self {
            state: Mutex::new(BudgetState {
                goodput_rate: p.initial_budget_rate,
                budget_us: p.initial_budget_rate * p.max_burst_secs,
                last_update: Instant::now(),
                rejection_ema: 0.0,
            }),
            completed_cost_us: AtomicU64::new(0),
        }
    }

    /// Record a successful child RPC completion (lock-free).
    pub(crate) fn record_completion(&self, est_child_cost: u64) {
        self.completed_cost_us
            .fetch_add(est_child_cost, Ordering::Relaxed);
    }

    /// Returns true if the request should be admitted.
    pub(crate) fn should_admit(&self, est_child_cost: u64) -> bool {
        let p = &PolicyParams::global().pred;
        let cost = est_child_cost as f64;

        let drained = self.completed_cost_us.swap(0, Ordering::Relaxed) as f64;

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        state.last_update = now;

        if elapsed > 0.0 {
            let instant_rate = drained / elapsed;
            let alpha = 1.0 - (-elapsed / p.tau).exp();
            state.goodput_rate += alpha * (instant_rate - state.goodput_rate);
        }

        let budget_rate = if state.rejection_ema > p.rejection_threshold {
            state.goodput_rate * (1.0 + p.probe_min)
        } else {
            p.initial_budget_rate
        };

        state.budget_us += budget_rate * elapsed;
        let max_budget = (budget_rate * p.max_burst_secs).max(cost * 2.0);
        if state.budget_us > max_budget {
            state.budget_us = max_budget;
        }

        let rejected = if state.budget_us >= cost {
            state.budget_us -= cost;
            false
        } else {
            true
        };
        state.rejection_ema +=
            p.rejection_alpha * ((if rejected { 1.0 } else { 0.0 }) - state.rejection_ema);
        !rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_controller_admits_with_budget() {
        let ac = PredictiveAdmission::new();
        assert!(ac.should_admit(1000));
    }

    #[test]
    fn test_admission_controller_rejects_when_budget_exhausted() {
        let ac = PredictiveAdmission::new();
        let mut rejected = false;
        for _ in 0..20 {
            if !ac.should_admit(100_000) {
                rejected = true;
                break;
            }
        }
        assert!(
            rejected,
            "should eventually reject when budget is exhausted"
        );
    }

    #[test]
    fn test_goodput_tracking_refills_budget() {
        let ac = PredictiveAdmission::new();
        while ac.should_admit(100_000) {}

        ac.record_completion(100_000);
        ac.record_completion(100_000);

        std::thread::sleep(std::time::Duration::from_millis(50));

        assert!(
            ac.should_admit(1000),
            "should admit after completions refill the budget"
        );
    }
}
