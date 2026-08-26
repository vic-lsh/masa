// Estimation layer — latency tracking, deadline tightening, reprioritization,
// and local deadline checks (ABORT_SLACK / SIGNAL_SLACK).
//
// Active when the `estimator` feature is enabled. Runs independently of the
// admission control layer (ac_pred / ac_rajomon / noop).

use std::task::Poll;

use masa_core::{Context, PriorityHint, RootMethod, ABORT_SLACK};
use tonic::{Code, CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use super::default_estimator::DefaultLatencyEstimator;
#[cfg(any(feature = "eval_oracle_continuation", feature = "eval_estimator_audit"))]
use super::state::AfterChildEstimates;
use super::state::{
    is_early_return_response, ChildRPCTracker, EstimationTracker, LatencyEstimators,
    RequestMetadataTracker,
};
use crate::MethodRegistry;

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level estimation state (shared across requests).
#[derive(Debug)]
pub(crate) struct EstimationServer {
    pub(crate) est: LatencyEstimators<DefaultLatencyEstimator>,
}

impl EstimationServer {
    pub(crate) fn new(_service_name: &'static str) -> Self {
        Self {
            est: LatencyEstimators::new(),
        }
    }
}

impl LayerServer for EstimationServer {}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request estimation layer state.
///
/// Tracks latency distributions, tightens child deadlines (when `sched_pred`
/// is enabled), handles local deadline checks (ABORT_SLACK aborts the request,
/// SIGNAL_SLACK only signals admission control), and manages response metadata
/// propagation.
#[derive(Debug)]
pub(crate) struct EstimationLayer {
    pub(crate) estimation: EstimationTracker<DefaultLatencyEstimator>,
    request_metadata: RequestMetadataTracker,
    rpc: CowGrpcMethod,
}

impl Layer for EstimationLayer {
    type Server = EstimationServer;
    type Child = EstimationChild;

    fn new(method: &CowGrpcMethod, server: &EstimationServer, ctx: &mut Context) -> Self {
        let resolved_method_id = MethodRegistry::global().get_or_register(method.clone());
        // Set root_method at ingress (hop_count == 0)
        if ctx.hop_count() == 0 {
            ctx.set_root_method(RootMethod {
                service: method.service().to_string(),
                method: method.method().to_string(),
            });
        }
        let root_method_id = ctx.root_method().map(|rm| {
            MethodRegistry::global()
                .get_or_register(CowGrpcMethod::new(rm.service.clone(), rm.method.clone()))
        });
        Self {
            estimation: EstimationTracker::new(
                resolved_method_id,
                root_method_id,
                server.est.clone(),
            ),
            request_metadata: RequestMetadataTracker::new(),
            rpc: method.clone(),
        }
    }

    /// Reprioritize the current task and check the local deadline
    /// (ABORT_SLACK aborts; SIGNAL_SLACK marks the soft signal).
    #[inline]
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        if ABORT_SLACK {
            let local_deadline = ctx.deadline();
            if local_deadline != 0 && masa_core::time_now() > local_deadline {
                return Err(Err(Status::new(
                    Code::DeadlineExceeded,
                    format!(
                        "/EarlyReturn?src={}::{}&reason=LocalDeadlineExceeded",
                        self.rpc.service(),
                        self.rpc.method(),
                    ),
                )));
            }
        }

        super::signal_slack::mark_if_late(ctx, &self.request_metadata);

        #[cfg(feature = "sched_pred")]
        {
            let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
            tokio::task::reprioritize(tokio::task::TaskPriority::new(remaining));
        }

        self.request_metadata.start_poll();
        Ok(())
    }

    /// Compute latency estimates, tighten child deadline, and begin child
    /// RPC tracking. Feasibility checks are handled by the admission layer.
    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        child_ctx: &mut EstimationChild,
        #[cfg_attr(
            not(any(feature = "eval_oracle_continuation", feature = "eval_estimator_audit")),
            allow(unused_variables)
        )]
        request: &mut tonic::Request<T>,
        child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        let child_tracker = self.estimation.begin_child(child_method_name);
        let time_left = ctx.e2e_deadline().saturating_sub(masa_core::time_now());
        let root = self
            .estimation
            .root_method_id
            .unwrap_or(self.estimation.resolved_method_id);
        #[cfg(feature = "eval_estimator_audit")]
        let learned_remaining_raw = self.estimation.est.est_after_child_wallclock_for_group(
            root,
            self.estimation.resolved_method_id,
            child_tracker.path_prefix,
            &child_tracker.base_signature,
            child_tracker.service_path_prefix,
            &child_tracker.base_service_signature,
            child_tracker.child_id,
            u64::MAX,
        );
        #[cfg(feature = "eval_estimator_audit")]
        let learned_remaining = AfterChildEstimates {
            full: learned_remaining_raw.full.min(time_left),
            mean: learned_remaining_raw.mean.min(time_left),
            floor: learned_remaining_raw.floor.min(time_left),
        };
        #[cfg(not(feature = "eval_estimator_audit"))]
        let learned_remaining = self.estimation.est.est_after_child_wallclock_for_group(
            root,
            self.estimation.resolved_method_id,
            child_tracker.path_prefix,
            &child_tracker.base_signature,
            child_tracker.service_path_prefix,
            &child_tracker.base_service_signature,
            child_tracker.child_id,
            time_left,
        );

        // Keep the estimator path active in both variants so its bookkeeping
        // and runtime overhead are controlled. The evaluation-only oracle
        // changes only the value consumed by priority/deadline derivation.
        #[cfg(feature = "eval_oracle_continuation")]
        let remaining = exact_continuation_estimate(request, child_method_name, time_left)?;
        #[cfg(feature = "eval_oracle_continuation")]
        let exact_priority_remaining = corrupt_exact_priority_order(
            remaining.full,
            ctx.slo(),
            ctx.gateway_entry(),
            crate::policy_params::PolicyParams::global()
                .pred
                .eval_oracle_order_corruption_probability,
        )?;
        #[cfg(not(feature = "eval_oracle_continuation"))]
        let remaining = learned_remaining;

        self.estimation
            .est
            .log_estimates(&child_tracker.key, &remaining);

        // Wallclock-time decay applied to BOTH the deadline-tightening floor
        // and the priority-tightening full estimate. Stale samples (no fresh
        // observation in TAU_DECAY_US) shrink toward zero — without this, a
        // floor inflated by sustained queueing keeps tightening child
        // deadlines indefinitely, causing `abort_slack` to kill mid-flight
        // requests that could have completed (the same metastable trap the
        // BCF check now avoids).
        let learned_decay = crate::layer::est::state::decay_factor(
            masa_core::time_now(),
            self.estimation
                .est
                .after_child_wallclock_last_obs(child_tracker.key),
        );
        #[cfg(feature = "eval_oracle_continuation")]
        let decay = {
            let _ = learned_decay;
            let _ = learned_remaining;
            1.0
        };
        #[cfg(not(feature = "eval_oracle_continuation"))]
        let decay = learned_decay;
        let decayed_full = (remaining.full as f64 * decay) as u64;
        let decayed_floor = (remaining.floor as f64 * decay) as u64;

        #[cfg(feature = "eval_oracle_continuation")]
        let decayed_priority = (exact_priority_remaining as f64 * decay) as u64;
        #[cfg(all(
            feature = "eval_estimator_audit",
            not(feature = "eval_oracle_continuation")
        ))]
        let (decayed_priority, learned_priority_scale) = {
            let scale = crate::policy_params::PolicyParams::global()
                .pred
                .eval_learned_priority_scale;
            (scale_learned_priority(decayed_full, scale)?, scale)
        };
        #[cfg(all(
            not(feature = "eval_estimator_audit"),
            not(feature = "eval_oracle_continuation")
        ))]
        let decayed_priority = decayed_full;
        #[cfg(all(feature = "eval_estimator_audit", feature = "eval_oracle_continuation"))]
        let learned_priority_scale = 1.0;

        let (deadline, prio_hint) =
            Self::child_deadline_and_prio(ctx, decayed_priority, decayed_full, decayed_floor);

        #[cfg(feature = "eval_estimator_audit")]
        log_estimator_audit(
            ctx,
            &self.rpc,
            child_method_name,
            request,
            &child_tracker.key,
            time_left,
            learned_remaining_raw,
            learned_remaining,
            learned_decay,
            learned_priority_scale,
            decayed_priority,
            decayed_floor,
            deadline,
            prio_hint,
        )?;

        child_rpc.deadline = deadline;
        child_rpc.prio_hint = prio_hint;

        child_ctx.child_tracker = Some(child_tracker);

        Ok(())
    }

    /// Record child RPC completion: track latencies, absorb metadata.
    #[inline]
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: &EstimationChild,
    ) -> Result<(), Status> {
        if let Some(child_tracker) = child_ctx.child_tracker.as_ref() {
            self.estimation
                .record_child_complete(child_tracker, response);
            self.request_metadata.absorb_child_meta(response);
        }

        #[cfg(feature = "sched_pred")]
        if let Err(status) = response {
            return Err(status.clone());
        }

        Ok(())
    }

    /// Stop compute tracking and check the local deadline.
    ///
    /// ABORT_SLACK can only replace a Pending poll with an early return.
    /// SIGNAL_SLACK records a soft signal on every post-poll check, including
    /// Ready, because it does not alter the response.
    #[inline]
    fn after_poll<Ret>(
        &self,
        ctx: &Context,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.request_metadata.end_poll();
        if let Poll::Pending = poll {
            if ABORT_SLACK {
                let local_deadline = ctx.deadline();
                if local_deadline != 0 && masa_core::time_now() > local_deadline {
                    return Err(Err(Status::new(
                        Code::DeadlineExceeded,
                        format!(
                            "/EarlyReturn?src={}::{}&reason=LocalDeadlineExceeded",
                            self.rpc.service(),
                            self.rpc.method(),
                        ),
                    )));
                }
            }
        }
        super::signal_slack::mark_if_late(ctx, &self.request_metadata);
        Ok(())
    }

    /// Flush estimation observations and build response metadata.
    ///
    /// Three response classes drive different bookkeeping:
    /// 1. `Err(EarlyReturn)` — abort path. Mark local early-return; skip
    ///    flush so the latency estimator only learns from on-time work.
    /// 2. `Ok` but the subtree tripped `signal_slack` — request finished
    ///    successfully, but its wallclock was inflated by the
    ///    signal-but-continue runtime. Skip flush for the same reason: an
    ///    inflated observation poisons the estimator, which then
    ///    over-tightens child deadlines in the next requests and triggers
    ///    even more signals.
    /// 3. `Ok` and on-time — the only case where the estimator should learn.
    #[inline]
    fn finalize<Ret>(&self, ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        if is_early_return_response(result) {
            self.request_metadata.mark_early_return();
        } else if !super::signal_slack::should_skip_flush(&self.request_metadata) {
            self.estimation.flush();
        }
        self.request_metadata.inject_response_meta(ctx);
    }
}

#[cfg(any(feature = "eval_oracle_continuation", feature = "eval_estimator_audit"))]
fn continuation_reference<T>(
    request: &tonic::Request<T>,
    child_method: &CowGrpcMethod,
) -> Result<u64, Status> {
    let key = masa_core::ORACLE_REMAINING_AFTER_US_HEADER;
    let value = request.metadata().get(key).ok_or_else(|| {
        Status::internal(format!(
            "missing exact-continuation header '{}' for {}::{}",
            key,
            child_method.service(),
            child_method.method()
        ))
    })?;
    let value = value.to_str().map_err(|e| {
        Status::internal(format!(
            "invalid exact-continuation header '{}' for {}::{}: {}",
            key,
            child_method.service(),
            child_method.method(),
            e
        ))
    })?;
    value.parse::<u64>().map_err(|e| {
        Status::internal(format!(
            "invalid exact-continuation header '{}' value '{}' for {}::{}: {}",
            key,
            value,
            child_method.service(),
            child_method.method(),
            e
        ))
    })
}

#[cfg(feature = "eval_estimator_audit")]
#[allow(clippy::too_many_arguments)]
fn log_estimator_audit<T>(
    ctx: &Context,
    parent_method: &CowGrpcMethod,
    child_method: &CowGrpcMethod,
    request: &tonic::Request<T>,
    key: &super::latency_map::ParentToChildKey,
    time_left_us: u64,
    learned_raw: AfterChildEstimates,
    learned: AfterChildEstimates,
    decay: f64,
    learned_priority_scale: f64,
    effective_priority_us: u64,
    effective_floor_us: u64,
    child_deadline: u64,
    priority: PriorityHint,
) -> Result<(), Status> {
    let reference_us = continuation_reference(request, child_method)?;
    let event = serde_json::json!({
        "timestamp_us": masa_core::time_now(),
        "request_id": ctx.request_id(),
        "api": ctx.api(),
        "slo_us": ctx.slo(),
        "gateway_entry_us": ctx.gateway_entry(),
        "e2e_deadline_us": ctx.e2e_deadline(),
        "parent_deadline_us": ctx.deadline(),
        "parent_method": format!("{}::{}", parent_method.service(), parent_method.method()),
        "child_method": format!("{}::{}", child_method.service(), child_method.method()),
        "effective_priority_key": key.to_string(),
        "time_left_us": time_left_us,
        "learned_full_raw_us": learned_raw.full,
        "learned_mean_raw_us": learned_raw.mean,
        "learned_floor_raw_us": learned_raw.floor,
        "learned_full_us": learned.full,
        "learned_mean_us": learned.mean,
        "learned_floor_us": learned.floor,
        "decay": decay,
        "learned_priority_scale": learned_priority_scale,
        "effective_priority_us": effective_priority_us,
        "effective_floor_us": effective_floor_us,
        "reference_remaining_us": reference_us,
        "child_deadline_us": child_deadline,
        "priority_hint": priority.value(),
    });
    log::info!("EST_AUDIT_JSON:{}", event);
    Ok(())
}

#[cfg(feature = "eval_oracle_continuation")]
fn exact_continuation_estimate<T>(
    request: &tonic::Request<T>,
    child_method: &CowGrpcMethod,
    time_left: u64,
) -> Result<AfterChildEstimates, Status> {
    let estimate = continuation_reference(request, child_method)?;
    let estimate = estimate.min(time_left);
    Ok(AfterChildEstimates {
        full: estimate,
        mean: estimate,
        floor: estimate,
    })
}

/// Scale only the learned continuation consumed by the soft scheduler.
#[cfg(all(
    feature = "eval_estimator_audit",
    not(feature = "eval_oracle_continuation")
))]
fn scale_learned_priority(remaining_us: u64, scale: f64) -> Result<u64, Status> {
    if !scale.is_finite() || scale < 0.0 {
        return Err(Status::internal(format!(
            "pred.eval_learned_priority_scale must be finite and non-negative, got {}",
            scale
        )));
    }

    let scaled = remaining_us as f64 * scale;
    if !scaled.is_finite() || scaled >= u64::MAX as f64 {
        return Ok(u64::MAX);
    }
    Ok(scaled as u64)
}

/// Deterministically reverse the binary continuation order for a sampled
/// fraction of matched-deadline bursts.
///
/// Complementing around the common end-to-end SLO makes a smaller exact
/// continuation produce a larger scheduling estimate and vice versa. The
/// returned value is used only as the soft scheduling signal. The exact
/// estimate bundle remains unchanged for hard deadline propagation, keeping
/// the intervention from changing admission, abortion, or request work.
#[cfg(feature = "eval_oracle_continuation")]
fn corrupt_exact_priority_order(
    exact_remaining: u64,
    slo_us: u64,
    gateway_entry: u64,
    probability: f64,
) -> Result<u64, Status> {
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err(Status::internal(format!(
            "pred.eval_oracle_order_corruption_probability must be in [0, 1], got {}",
            probability
        )));
    }

    // A zero continuation is the terminal child in this experiment, not one
    // of the binary short/long choices at the shared bottleneck. Leaving it
    // alone prevents the intervention from perturbing downstream leaf queues.
    if exact_remaining > 0 && deterministic_sample(gateway_entry, probability) {
        return Ok(slo_us.saturating_sub(exact_remaining));
    }
    Ok(exact_remaining)
}

#[cfg(feature = "eval_oracle_continuation")]
fn deterministic_sample(key: u64, probability: f64) -> bool {
    if probability <= 0.0 {
        return false;
    }
    if probability >= 1.0 {
        return true;
    }

    // SplitMix64 provides a stable, well-distributed choice without runtime
    // RNG state. A shared gateway-entry timestamp deliberately gives every
    // request in a matched-deadline burst the same choice.
    let mut mixed = key.wrapping_add(0x9e37_79b9_7f4a_7c15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;

    let unit = mixed as f64 / (u64::MAX as f64 + 1.0);
    unit < probability
}

impl EstimationLayer {
    /// Compute child deadline and priority hint.
    ///
    /// When `sched_pred` is enabled, tightens the deadline by subtracting
    /// `est_remaining`. When disabled, passes through the parent values.
    #[inline]
    fn child_deadline_and_prio(
        ctx: &Context,
        #[cfg_attr(not(feature = "sched_pred"), allow(unused_variables))]
        priority_est_remaining: u64,
        deadline_full_est_remaining: u64,
        deadline_est_remaining: u64,
    ) -> (u64, PriorityHint) {
        #[cfg(feature = "sched_pred")]
        {
            // Priority is a soft scheduling signal, so use the full estimate.
            // The propagated deadline is a hard abort threshold; use the floor
            // estimate to avoid converting estimator variance into false ERs.
            let hard_deadline_estimate =
                hard_deadline_estimate(deadline_full_est_remaining, deadline_est_remaining);
            let deadline = ctx.deadline().saturating_sub(hard_deadline_estimate);
            let priority_deadline = ctx.deadline().saturating_sub(priority_est_remaining);
            let priority_remaining = priority_deadline.saturating_sub(masa_core::time_now());
            (deadline, PriorityHint::new(priority_remaining))
        }
        #[cfg(not(feature = "sched_pred"))]
        {
            let _ = deadline_full_est_remaining;
            let d = ctx.deadline().saturating_sub(deadline_est_remaining);
            (d, ctx.prio_hint())
        }
    }
}

#[inline]
#[cfg(any(feature = "sched_pred", test))]
fn hard_deadline_estimate(slack_estimate: u64, deadline_estimate: u64) -> u64 {
    if cfg!(feature = "deadline_equals_slack") {
        slack_estimate
    } else {
        deadline_estimate
    }
}

#[cfg(test)]
mod tests {
    use super::hard_deadline_estimate;

    #[cfg(feature = "eval_oracle_continuation")]
    #[test]
    fn exact_continuation_reads_and_caps_header() {
        let mut request = tonic::Request::new(());
        request.metadata_mut().insert(
            masa_core::ORACLE_REMAINING_AFTER_US_HEADER,
            "12000".parse().unwrap(),
        );
        let method = tonic::CowGrpcMethod::new("Tail", "Run");

        let estimate = super::exact_continuation_estimate(&request, &method, 10_000).unwrap();
        assert_eq!(estimate.full, 10_000);
        assert_eq!(estimate.mean, 10_000);
        assert_eq!(estimate.floor, 10_000);
    }

    #[cfg(feature = "eval_oracle_continuation")]
    #[test]
    fn exact_continuation_requires_header() {
        let request = tonic::Request::new(());
        let method = tonic::CowGrpcMethod::new("Tail", "Run");

        let status = super::exact_continuation_estimate(&request, &method, 10_000).unwrap_err();
        assert!(status
            .message()
            .contains("missing exact-continuation header"));
    }

    #[cfg(feature = "eval_oracle_continuation")]
    #[test]
    fn exact_priority_corruption_reverses_binary_order_only() {
        let short = super::corrupt_exact_priority_order(2_000, 50_000, 7, 1.0).unwrap();
        let long = super::corrupt_exact_priority_order(25_000, 50_000, 7, 1.0).unwrap();

        assert!(short > long);
        assert_eq!(
            super::corrupt_exact_priority_order(0, 50_000, 7, 1.0).unwrap(),
            0
        );
    }

    #[cfg(feature = "eval_oracle_continuation")]
    #[test]
    fn exact_priority_corruption_zero_is_noop_and_invalid_probability_fails() {
        let unchanged = super::corrupt_exact_priority_order(25_000, 50_000, 7, 0.0).unwrap();
        assert_eq!(unchanged, 25_000);
        assert!(super::corrupt_exact_priority_order(25_000, 50_000, 7, 1.1).is_err());
    }

    #[cfg(feature = "eval_oracle_continuation")]
    #[test]
    fn exact_priority_corruption_sampling_is_deterministic_and_calibrated() {
        assert_eq!(
            super::deterministic_sample(42, 0.25),
            super::deterministic_sample(42, 0.25)
        );

        let selected = (0..10_000)
            .filter(|key| super::deterministic_sample(*key, 0.25))
            .count();
        assert!((2_350..=2_650).contains(&selected));
    }

    #[cfg(all(
        feature = "eval_estimator_audit",
        not(feature = "eval_oracle_continuation")
    ))]
    #[test]
    fn learned_priority_scale_is_isolated_and_saturating() {
        assert_eq!(super::scale_learned_priority(2_000, 2.5).unwrap(), 5_000);
        assert_eq!(super::scale_learned_priority(2_000, 0.0).unwrap(), 0);
        assert_eq!(
            super::scale_learned_priority(u64::MAX, 2.0).unwrap(),
            u64::MAX
        );
    }

    #[cfg(all(
        feature = "eval_estimator_audit",
        not(feature = "eval_oracle_continuation")
    ))]
    #[test]
    fn learned_priority_scale_rejects_invalid_values() {
        assert!(super::scale_learned_priority(2_000, -0.1).is_err());
        assert!(super::scale_learned_priority(2_000, f64::NAN).is_err());
        assert!(super::scale_learned_priority(2_000, f64::INFINITY).is_err());
    }

    #[cfg(feature = "deadline_equals_slack")]
    #[test]
    fn tied_deadline_uses_slack_estimate() {
        assert_eq!(hard_deadline_estimate(90, 40), 90);
    }

    #[cfg(not(feature = "deadline_equals_slack"))]
    #[test]
    fn default_deadline_uses_conservative_estimate() {
        assert_eq!(hard_deadline_estimate(90, 40), 40);
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

/// Per-child-RPC estimation layer state.
#[derive(Debug, Clone)]
pub(crate) struct EstimationChild {
    pub(crate) child_tracker: Option<ChildRPCTracker>,
}

impl LayerChild for EstimationChild {
    fn new() -> Self {
        Self {
            child_tracker: None,
        }
    }
}
