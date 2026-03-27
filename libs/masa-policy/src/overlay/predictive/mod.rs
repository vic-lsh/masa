// Predictive overlay — latency estimation, deadline tightening, and
// predictive admission control.
//
// When `est` is enabled, the overlay tracks latency distributions, tightens
// child deadlines (when `sched_pred` is also enabled), and runs predictive
// admission control. All state is wrapped here so that `standard.rs` can
// call methods unconditionally without `#[cfg]` annotations at every call site.

pub(crate) mod ac;
pub(crate) mod est;

use std::sync::Arc;
use std::task::Poll;

use masa_core::{Context, PriorityHint};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::{ChildRpcContext, Overlay, OverlayChild, OverlayServer};
use crate::MethodRegistry;
use ac::PredictiveAc;
use est::estimator::DefaultLatencyEstimator;
use est::state::{is_early_return_response, EstChildState, EstRequestState, EstServerState};

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level predictive overlay state (shared across requests).
#[derive(Debug)]
pub(crate) struct PredictiveOverlayServer {
    est: Arc<EstServerState<DefaultLatencyEstimator>>,
    pred_ac: Arc<PredictiveAc>,
}

impl OverlayServer for PredictiveOverlayServer {
    fn new() -> Self {
        Self {
            est: Arc::new(EstServerState::new()),
            pred_ac: Arc::new(PredictiveAc::new()),
        }
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request predictive overlay state.
#[derive(Debug)]
pub(crate) struct PredictiveOverlay {
    pub(crate) est: EstRequestState<DefaultLatencyEstimator>,
    pred_ac: Arc<PredictiveAc>,
    rpc: CowGrpcMethod,
}

impl Overlay for PredictiveOverlay {
    type Server = PredictiveOverlayServer;
    type Child = PredictiveOverlayChild;

    fn new(method: &CowGrpcMethod, server: &PredictiveOverlayServer, _ctx: &mut Context) -> Self {
        let resolved_method_id =
            MethodRegistry::global().get_or_register_method(method.service(), method.method());
        Self {
            est: EstRequestState::new(resolved_method_id, server.est.clone()),
            pred_ac: server.pred_ac.clone(),
            rpc: method.clone(),
        }
    }

    /// Reprioritize the current task based on remaining time to deadline.
    #[inline]
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(feature = "sched_pred")]
        {
            let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
            tokio::task::reprioritize(PriorityHint::new(remaining));
        }
        #[cfg(not(feature = "sched_pred"))]
        let _ = ctx;

        self.est.start_compute_tracking();
        Ok(())
    }

    /// Compute child deadline and priority, run predictive admission control,
    /// and set the child request context.
    ///
    /// Returns `Err` if the request should be shed.
    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        child_ctx: &mut PredictiveOverlayChild,
        _request: &mut tonic_core::Request<T>,
        child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        let est_remaining = {
            let result =
                self.est
                    .prepare_before_child_rpc(ctx, child_method_name, &mut child_ctx.est);

            if self.pred_ac.admission_check(
                &self.est.server,
                self.est.resolved_method_id,
                ctx,
                result.key,
            ) {
                return Err(Status::new(
                    Code::DeadlineExceeded,
                    format!(
                        "/EarlyReturn?src={}::{}?last_rpc={}::{}",
                        self.rpc.service(),
                        self.rpc.method(),
                        child_method_name.service(),
                        child_method_name.method(),
                    ),
                ));
            }

            result.est_remaining
        };

        let (deadline, prio_hint) = Self::child_deadline_and_prio(ctx, est_remaining);
        child_rpc.deadline = deadline;
        child_rpc.prio_hint = prio_hint;

        Ok(())
    }

    /// Process a child RPC response: track latencies, propagate errors.
    #[inline]
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: &PredictiveOverlayChild,
    ) -> Result<(), Status> {
        child_ctx.est.finalize(response);
        if let Some(downstream_util) = self.est.after_child_rpc(response, &child_ctx.est) {
            self.pred_ac.update_bottleneck(ctx.api(), downstream_util);
        }

        #[cfg(feature = "sched_pred")]
        if let Err(status) = response {
            return Err(status.clone());
        }

        Ok(())
    }

    /// Stop compute tracking after a poll.
    #[inline]
    fn after_poll<Ret>(
        &self,
        _ctx: &Context,
        _poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.est.stop_compute_tracking();
        Ok(())
    }

    /// Track latencies and inject response metadata at finalization.
    ///
    /// Sets `response_meta` on the shared `ctx`; the caller serializes once.
    #[inline]
    fn finalize<Ret>(&self, ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.est.track_latencies();
        }
        self.est.inject_response_meta(ctx);
    }
}

impl PredictiveOverlay {
    /// Compute child deadline and priority hint.
    ///
    /// When `sched_pred` is enabled, tightens the deadline by subtracting
    /// `est_remaining`.  Uses deadline alone as `prio_hint` (not
    /// `deadline - est_child`) to avoid priority inversions under load.
    ///
    /// When `sched_pred` is disabled, passes through the parent values.
    #[inline]
    fn child_deadline_and_prio(ctx: &Context, est_remaining: u64) -> (u64, PriorityHint) {
        #[cfg(feature = "sched_pred")]
        {
            let d = ctx.deadline().saturating_sub(est_remaining);
            (d, PriorityHint::new(d))
        }
        #[cfg(not(feature = "sched_pred"))]
        {
            let _ = est_remaining;
            (ctx.deadline(), ctx.prio_hint())
        }
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

/// Per-child-RPC predictive overlay state.
#[derive(Debug, Clone)]
pub(crate) struct PredictiveOverlayChild {
    pub(crate) est: EstChildState<DefaultLatencyEstimator>,
}

impl OverlayChild for PredictiveOverlayChild {
    fn new() -> Self {
        Self {
            est: EstChildState::new(),
        }
    }
}
