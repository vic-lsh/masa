// Common hook state shared by all scheduling policies (standard, est_abort).
//
// `BaseHookState` holds the fields and logic common to every policy's
// `ParentContext`: SLO abort checking, Rajomon admission control, queue
// latency tracking, and context propagation. Policy-specific logic
// (e.g., est_abort's reprioritization) lives in the policy files.

use crate::{masa::context::read_context, CowGrpcMethod, GrpcMethod, Response, Status};

use super::common::{QueueLatencyTracker, SloAbortHandler};
use super::rajomon::RajomonHandler;
use super::resolve_method_name_from_http;
use masa_core::Context;

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct BaseHookState {
    pub(super) ctx: Context,
    pub(super) resolved_method: CowGrpcMethod,
    pub(super) q_lat_tracker: QueueLatencyTracker,
    pub(super) slo_abort: SloAbortHandler,
    pub(super) rajomon: RajomonHandler,
}

impl BaseHookState {
    /// Initialize common hook state from an incoming HTTP request.
    pub(super) fn new<B>(method: GrpcMethod, req: &http::Request<B>) -> Self {
        let mut ctx = read_context(req);
        let resolved_method = resolve_method_name_from_http(method, req);
        let mut rajomon = RajomonHandler::new(resolved_method.clone());
        rajomon.check_inbound(&mut ctx);

        Self {
            ctx,
            resolved_method: resolved_method.clone(),
            q_lat_tracker: QueueLatencyTracker::new(),
            slo_abort: SloAbortHandler::new(resolved_method),
            rajomon,
        }
    }

    /// Check rajomon drop and SLO abort guards. Returns early-return error envelope
    /// on failure. Used in `before_poll` and similar contexts.
    pub(super) fn check_guards<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.rajomon.should_drop() {
            return Err(Err(self.rajomon.issue_error(None)));
        }
        if self.slo_abort.check(&self.ctx) {
            return Err(Err(self.slo_abort.issue_error()));
        }
        Ok(())
    }

    /// Check rajomon drop and SLO abort guards, returning a flat `Status` error.
    /// Used in `before_child_rpc`.
    pub(super) fn check_guards_status(&self) -> Result<(), Status> {
        if self.rajomon.should_drop() {
            return Err(self.rajomon.issue_error(None));
        }
        if self.slo_abort.check(&self.ctx) {
            return Err(self.slo_abort.issue_error());
        }
        Ok(())
    }

    /// Track queue delay and poll latency. Called after guard checks in `before_poll`.
    pub(super) fn track_poll(&self) {
        self.rajomon.track_queue_delay();
        self.q_lat_tracker.track_poll();
    }

    /// Check guards on `Poll::Pending` in `after_poll`.
    pub(super) fn check_pending_guards<Ret>(
        &self,
        poll: &std::task::Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let std::task::Poll::Pending = poll {
            self.check_guards()?;
        }
        Ok(())
    }

    /// Common finalization: rajomon queue delay, queue latency injection, price injection.
    pub(super) fn finalize<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        self.rajomon.finalize_queue_delay();
        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
        self.rajomon.inject_price_to_response(result);
    }

    /// Update rajomon cache and SLO abort handler after a child RPC completes.
    pub(super) fn update_after_child<T>(
        &self,
        child_method: &CowGrpcMethod,
        response: &Result<Response<T>, Status>,
    ) {
        if let Ok(resp) = response {
            self.rajomon
                .update_cache_from_response(child_method, resp.metadata());
        } else if let Err(status) = response {
            self.rajomon
                .update_cache_from_response(child_method, status.metadata());
        }
        self.slo_abort.set_last_child(child_method.clone());
    }
}
