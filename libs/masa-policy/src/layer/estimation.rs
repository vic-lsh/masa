// Estimation layer — latency tracking, deadline tightening, reprioritization,
// and ABORT_SLACK local deadline checks.
//
// Active when the `estimator` feature is enabled. Runs independently of the
// admission control layer (ac_pred / ac_rajomon / noop).

use std::sync::OnceLock;
use std::task::Poll;

use masa_core::{Context, PriorityHint, RootMethod, ABORT_SLACK};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::est::estimator::DefaultLatencyEstimator;
use super::est::state::{
    is_early_return_response, ChildRPCTracker, EstimationTracker, LatencyEstimators,
    RequestMetadataTracker,
};
use super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use crate::MethodRegistry;

// ── Global estimator access ─────────────────────────────────────────────

/// Global handle to the shared `LatencyEstimators`, set once when
/// `EstimationServer` is created. Allows the admission layer to look up
/// cost estimates without a direct reference to the estimation layer.
static GLOBAL_ESTIMATORS: OnceLock<LatencyEstimators<DefaultLatencyEstimator>> = OnceLock::new();

/// Returns a clone of the global `LatencyEstimators`.
///
/// Panics if called before `EstimationServer::new()` — in practice this
/// never happens because the server context initialises all layers before
/// any request arrives.
#[cfg(feature = "ac_pred")]
pub(crate) fn global_estimators() -> LatencyEstimators<DefaultLatencyEstimator> {
    GLOBAL_ESTIMATORS
        .get()
        .expect("EstimationServer must be initialised before admission layer")
        .clone()
}

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level estimation state (shared across requests).
#[derive(Debug)]
pub(crate) struct EstimationServer {
    pub(crate) est: LatencyEstimators<DefaultLatencyEstimator>,
}

impl LayerServer for EstimationServer {
    fn new() -> Self {
        let est = LatencyEstimators::new();
        // Publish for the admission layer to access cost estimates.
        let _ = GLOBAL_ESTIMATORS.set(est.clone());
        Self { est }
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request estimation layer state.
///
/// Tracks latency distributions, tightens child deadlines (when `sched_pred`
/// is enabled), handles ABORT_SLACK local deadline checks, and manages
/// response metadata propagation.
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
            ctx.root_method = Some(RootMethod {
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

    /// Reprioritize the current task and check ABORT_SLACK local deadline.
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

        #[cfg(feature = "sched_pred")]
        {
            let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
            tokio::task::reprioritize(PriorityHint::new(remaining));
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
        _request: &mut tonic_core::Request<T>,
        child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        let child_tracker = self.estimation.begin_child(child_method_name);
        let time_left = ctx.e2e_deadline().saturating_sub(masa_core::time_now());
        let root = self
            .estimation
            .root_method_id
            .unwrap_or(self.estimation.resolved_method_id);
        let remaining = self.estimation.est.est_after_child_wallclock_for_group(
            root,
            self.estimation.resolved_method_id,
            child_tracker.path_prefix,
            &child_tracker.base_signature,
            child_tracker.service_path_prefix,
            &child_tracker.base_service_signature,
            child_tracker.child_id,
            time_left,
        );

        self.estimation
            .est
            .log_estimates(&child_tracker.key, &remaining);

        let (deadline, prio_hint) =
            Self::child_deadline_and_prio(ctx, remaining.full, remaining.floor);
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

    /// Stop compute tracking and check ABORT_SLACK local deadline on Pending.
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
        Ok(())
    }

    /// Flush estimation observations and build response metadata.
    #[inline]
    fn finalize<Ret>(&self, ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        if is_early_return_response(result) {
            self.request_metadata.mark_early_return();
        } else {
            self.estimation.flush();
        }
        self.request_metadata.inject_response_meta(ctx);
    }
}

impl EstimationLayer {
    /// Compute child deadline and priority hint.
    ///
    /// When `sched_pred` is enabled, tightens the deadline by subtracting
    /// `est_remaining`. When disabled, passes through the parent values.
    #[inline]
    fn child_deadline_and_prio(
        ctx: &Context,
        priority_est_remaining: u64,
        deadline_est_remaining: u64,
    ) -> (u64, PriorityHint) {
        #[cfg(feature = "sched_pred")]
        {
            // Priority is a soft scheduling signal, so use the full estimate.
            // The propagated deadline is a hard abort threshold; use the floor
            // estimate to avoid converting estimator variance into false ERs.
            let deadline = ctx.deadline().saturating_sub(deadline_est_remaining);
            let priority_deadline = ctx.deadline().saturating_sub(priority_est_remaining);
            (deadline, PriorityHint::new(priority_deadline))
        }
        #[cfg(not(feature = "sched_pred"))]
        {
            let d = ctx.deadline().saturating_sub(deadline_est_remaining);
            (d, ctx.prio_hint())
        }
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
