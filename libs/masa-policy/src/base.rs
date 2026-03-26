// Common hook state shared by all scheduling policies.
//
// `BaseHookState` holds the fields and logic common to every policy's
// `ParentContext`: SLO abort checking, queue latency tracking, and context
// propagation. Policy-specific logic (estimation, admission control) lives
// in the overlay.

use crate::slo_abort::SloAbortHandler;
use masa_core::Context;
use tonic_core::{CowGrpcMethod, Response, Status};

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct BaseHookState {
    pub(super) ctx: Context,
    pub(super) resolved_method: CowGrpcMethod,
    pub(super) q_lat_tracker: QueueLatencyTracker,
    pub(super) slo_abort: SloAbortHandler,
}

impl BaseHookState {
    /// Initialize common hook state from pre-extracted context and method.
    pub(super) fn new(ctx: Context, resolved_method: CowGrpcMethod) -> Self {
        Self {
            slo_abort: SloAbortHandler::new(resolved_method.clone()),
            q_lat_tracker: QueueLatencyTracker::new(),
            ctx,
            resolved_method,
        }
    }

    /// Check SLO abort guard. Returns early-return error envelope on failure.
    pub(super) fn check_guards<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.slo_abort.check(&self.ctx) {
            return Err(Err(self.slo_abort.issue_error()));
        }
        Ok(())
    }

    /// Check SLO abort guard, returning a flat `Status` error.
    /// Used in `before_child_rpc`.
    pub(super) fn check_guards_status(&self) -> Result<(), Status> {
        self.check_guards::<()>().map_err(|e| e.unwrap_err())
    }

    /// Track queue latency. Called after guard checks in `before_poll`.
    pub(super) fn track_poll(&self) {
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

    /// Common finalization: queue latency injection.
    pub(super) fn finalize<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
    }

    /// Update SLO abort handler after a child RPC completes.
    pub(super) fn update_after_child(&self, child_method: &CowGrpcMethod) {
        self.slo_abort.set_last_child(child_method.clone());
    }
}

// ── Queue Latency Tracker ───────────────────────────────────────────────

#[cfg(feature = "trace-queue")]
use masa_core::QueueLatencies;
#[cfg(feature = "trace-queue")]
use std::sync::atomic::AtomicU64;
#[cfg(feature = "trace-queue")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "trace-queue")]
#[derive(Debug)]
pub(crate) struct QueueLatencyTracker {
    initial_q_lat: AtomicU64,
    resume_q_lat: AtomicU64,
    is_first_poll: AtomicBool,
}

#[cfg(feature = "trace-queue")]
impl Default for QueueLatencyTracker {
    fn default() -> Self {
        Self {
            initial_q_lat: AtomicU64::new(0),
            resume_q_lat: AtomicU64::new(0),
            is_first_poll: AtomicBool::new(true),
        }
    }
}

#[cfg(feature = "trace-queue")]
impl QueueLatencyTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn track_poll(&self) {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        if queue_latency > 0 {
            if self.is_first_poll.swap(false, Ordering::Relaxed) {
                self.initial_q_lat
                    .fetch_add(queue_latency, Ordering::AcqRel);
            } else {
                self.resume_q_lat.fetch_add(queue_latency, Ordering::AcqRel);
            }
        }
    }

    pub(crate) fn track_child_response<T>(&self, response: &Result<Response<T>, Status>) {
        if let Ok(resp) = response {
            use crate::context_ext::MasaResponseExt;
            if let Some(ctx) = resp.get_masa_context() {
                if let Some(ql) = ctx.queue_latencies {
                    self.initial_q_lat.fetch_add(ql.initial, Ordering::AcqRel);
                    self.resume_q_lat.fetch_add(ql.resume, Ordering::AcqRel);
                }
            }
        }
    }

    pub(crate) fn inject_context_metadata<T>(
        &self,
        ctx: &Context,
        result: &mut Result<Response<T>, Status>,
    ) {
        use crate::context_ext::{MasaResponseExt, MasaStatusExt};

        let mut ctx = ctx.clone();

        let initial = self.initial_q_lat.load(Ordering::Acquire);
        let resume = self.resume_q_lat.load(Ordering::Acquire);
        ctx.queue_latencies = Some(QueueLatencies { initial, resume });

        match result {
            Ok(resp) => resp.set_masa_context(&ctx),
            Err(status) => status.set_masa_context(&ctx),
        };
    }
}

#[cfg(not(feature = "trace-queue"))]
#[derive(Debug, Default)]
pub(crate) struct QueueLatencyTracker;

#[cfg(not(feature = "trace-queue"))]
impl QueueLatencyTracker {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn track_poll(&self) {}

    pub(crate) fn track_child_response<T>(&self, _response: &Result<Response<T>, Status>) {}

    pub(crate) fn inject_context_metadata<T>(
        &self,
        _ctx: &Context,
        _result: &mut Result<Response<T>, Status>,
    ) {
    }
}
