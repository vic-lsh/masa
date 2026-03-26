// Zero-cost overlay for predictive scheduling and estimation.
//
// `PredictiveOverlay` wraps all `est`-gated state (latency estimation,
// predictive admission control, and deadline tightening) into a single
// abstraction so that `standard.rs` can call methods unconditionally
// without `#[cfg]` annotations at every call site.
//
// When `est` is enabled, the overlay performs latency tracking, deadline
// tightening (`sched_pred`), reprioritization, and predictive admission
// control.  When disabled, every method compiles away to a no-op.

// ── est ENABLED ─────────────────────────────────────────────────────────────

#[cfg(feature = "est")]
mod enabled {
    use std::sync::Arc;
    use std::task::Poll;

    use masa_core::{Context, ContextBuilder, PriorityHint};
    use tonic_core::{CowGrpcMethod, Response, Status};

    use crate::ac::predictive_ac::PredictiveAc;
    use crate::est::estimator::DefaultLatencyEstimator;
    use crate::est::state::{is_early_return_response, EstChildState, EstRequestState, EstServerState};
    use crate::MethodRegistry;

    /// Server-level predictive overlay state (shared across requests).
    #[derive(Debug)]
    pub(crate) struct PredictiveOverlayServer {
        pub(super) est: Arc<EstServerState<DefaultLatencyEstimator>>,
        pub(super) pred_ac: Arc<PredictiveAc>,
    }

    impl PredictiveOverlayServer {
        pub(crate) fn new() -> Self {
            Self {
                est: Arc::new(EstServerState::new()),
                pred_ac: Arc::new(PredictiveAc::new()),
            }
        }
    }

    /// Per-request predictive overlay state.
    #[derive(Debug)]
    pub(crate) struct PredictiveOverlay {
        pub(crate) est: EstRequestState<DefaultLatencyEstimator>,
        pred_ac: Arc<PredictiveAc>,
    }

    impl PredictiveOverlay {
        pub(crate) fn new(
            method: &CowGrpcMethod,
            server: &PredictiveOverlayServer,
        ) -> Self {
            let resolved_method_id = MethodRegistry::global()
                .get_or_register_method(method.service(), method.method());
            Self {
                est: EstRequestState::new(resolved_method_id, server.est.clone()),
                pred_ac: server.pred_ac.clone(),
            }
        }

        /// Reprioritize the current task based on remaining time to deadline.
        #[inline]
        pub(crate) fn before_poll(&self, ctx: &Context) {
            #[cfg(feature = "sched_pred")]
            {
                let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
                tokio::task::reprioritize(PriorityHint::new(remaining));
            }
            #[cfg(not(feature = "sched_pred"))]
            let _ = ctx;

            self.est.start_compute_tracking();
        }

        /// Compute child deadline and priority, run predictive admission control,
        /// and set the child request context.
        ///
        /// Returns `Err` if the request should be shed.
        #[inline]
        pub(crate) fn before_child_rpc<T>(
            &self,
            ctx: &Context,
            child_method_name: &CowGrpcMethod,
            child_ctx: &mut super::PredictiveOverlayChild,
            _request: &mut tonic_core::Request<T>,
            builder: ContextBuilder,
            slo_abort_error: impl FnOnce() -> Status,
        ) -> Result<(u64, PriorityHint, ContextBuilder), Status> {
            let est_remaining = {
                let result = self.est.prepare_before_child_rpc(
                    ctx,
                    child_method_name,
                    &mut child_ctx.est,
                );

                if self.pred_ac.admission_check(
                    &self.est.server,
                    self.est.resolved_method_id,
                    ctx,
                    result.key,
                ) {
                    return Err(slo_abort_error());
                }

                result.est_remaining
            };

            let (deadline, prio_hint) = Self::child_deadline_and_prio(ctx, est_remaining);

            let builder = builder
                .deadline(deadline)
                .prio_hint(prio_hint)
                .hop_count(ctx.hop_count().saturating_add(1));

            Ok((deadline, prio_hint, builder))
        }

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

        /// Process a child RPC response: track latencies, propagate errors.
        #[inline]
        pub(crate) fn after_child_rpc<T>(
            &self,
            ctx: &Context,
            response: &mut Result<Response<T>, Status>,
            child_ctx: &super::PredictiveOverlayChild,
        ) -> Result<(), Status> {
            child_ctx.est.finalize(response);
            if let Some(downstream_util) = self.est.after_child_rpc(response, &child_ctx.est) {
                self.pred_ac
                    .update_bottleneck(ctx.api(), downstream_util);
            }

            #[cfg(feature = "sched_pred")]
            if let Err(status) = response {
                return Err(status.clone());
            }

            Ok(())
        }

        /// Stop compute tracking after a poll.
        #[inline]
        pub(crate) fn after_poll<Ret>(&self, _poll: &Poll<Result<Response<Ret>, Status>>) {
            self.est.stop_compute_tracking();
        }

        /// Track latencies and inject response metadata at finalization.
        #[inline]
        pub(crate) fn finalize<Ret>(
            &self,
            ctx: &Context,
            result: &mut Result<Response<Ret>, Status>,
        ) {
            if !is_early_return_response(result) {
                self.est.track_latencies();
            }
            self.est.inject_response_meta(ctx, result);
        }
    }

    /// Per-child-RPC predictive overlay state.
    #[derive(Debug, Clone)]
    pub(crate) struct PredictiveOverlayChild {
        pub(crate) est: EstChildState<DefaultLatencyEstimator>,
    }

    impl PredictiveOverlayChild {
        pub(crate) fn new() -> Self {
            Self {
                est: EstChildState::new(),
            }
        }
    }
}

// ── est DISABLED ────────────────────────────────────────────────────────────

#[cfg(not(feature = "est"))]
mod disabled {
    use std::task::Poll;

    use masa_core::{Context, ContextBuilder, PriorityHint};
    use tonic_core::{CowGrpcMethod, Response, Status};

    /// Server-level predictive overlay state (no-op when `est` is disabled).
    #[derive(Debug)]
    pub(crate) struct PredictiveOverlayServer;

    impl PredictiveOverlayServer {
        pub(crate) fn new() -> Self {
            Self
        }
    }

    /// Per-request predictive overlay state (no-op when `est` is disabled).
    #[derive(Debug)]
    pub(crate) struct PredictiveOverlay;

    impl PredictiveOverlay {
        pub(crate) fn new(
            _method: &CowGrpcMethod,
            _server: &PredictiveOverlayServer,
        ) -> Self {
            Self
        }

        #[inline]
        pub(crate) fn before_poll(&self, _ctx: &Context) {}

        #[inline]
        pub(crate) fn before_child_rpc<T>(
            &self,
            ctx: &Context,
            _child_method_name: &CowGrpcMethod,
            _child_ctx: &mut super::PredictiveOverlayChild,
            _request: &mut tonic_core::Request<T>,
            builder: ContextBuilder,
            _slo_abort_error: impl FnOnce() -> Status,
        ) -> Result<(u64, PriorityHint, ContextBuilder), Status> {
            Ok((ctx.deadline(), ctx.prio_hint(), builder.deadline(ctx.deadline()).prio_hint(ctx.prio_hint())))
        }

        #[inline]
        pub(crate) fn after_child_rpc<T>(
            &self,
            _ctx: &Context,
            _response: &mut Result<Response<T>, Status>,
            _child_ctx: &super::PredictiveOverlayChild,
        ) -> Result<(), Status> {
            Ok(())
        }

        #[inline]
        pub(crate) fn after_poll<Ret>(&self, _poll: &Poll<Result<Response<Ret>, Status>>) {}

        #[inline]
        pub(crate) fn finalize<Ret>(
            &self,
            _ctx: &Context,
            _result: &mut Result<Response<Ret>, Status>,
        ) {
        }
    }

    /// Per-child-RPC predictive overlay state (no-op when `est` is disabled).
    #[derive(Debug, Clone)]
    pub(crate) struct PredictiveOverlayChild;

    impl PredictiveOverlayChild {
        pub(crate) fn new() -> Self {
            Self
        }
    }
}

// Re-export the active implementation.
#[cfg(feature = "est")]
pub(crate) use enabled::{PredictiveOverlay, PredictiveOverlayChild, PredictiveOverlayServer};
#[cfg(not(feature = "est"))]
pub(crate) use disabled::{PredictiveOverlay, PredictiveOverlayChild, PredictiveOverlayServer};
