use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Mutex,
};
use std::time::Instant;

use masa_core::{Context, ResponseMeta};
use tonic_core::{Code, Response, Status};

use crate::context_ext::MasaResponseExt;

// ══════════════════════════════════════════════════════════════════════════
// Response metadata assembly
// ══════════════════════════════════════════════════════════════════════════

/// Information extracted from a child RPC response.
#[allow(dead_code)]
pub(crate) struct ChildResponseInfo {
    pub downstream_util: Option<f32>,
    pub accumulated_compute_us: Option<u64>,
}

/// Tracks cumulative compute time (CPU time spent in poll) for a single request.
#[derive(Debug)]
pub(crate) struct ComputeTracker {
    /// Accumulated compute microseconds across all polls.
    poll_compute_us: AtomicU64,
    /// Start instant of the current poll (`None` when not inside a poll).
    poll_start: Mutex<Option<Instant>>,
}

impl ComputeTracker {
    pub(super) fn new() -> Self {
        Self {
            poll_compute_us: AtomicU64::new(0),
            poll_start: Mutex::new(None),
        }
    }

    /// Start tracking compute time for the current poll.
    pub(super) fn start(&self) {
        *self.poll_start.lock().unwrap() = Some(Instant::now());
    }

    /// Stop tracking compute time and accumulate elapsed time.
    pub(super) fn stop(&self) {
        if let Some(start) = self.poll_start.lock().unwrap().take() {
            let elapsed_us = start.elapsed().as_micros() as u64;
            self.poll_compute_us
                .fetch_add(elapsed_us, Ordering::Relaxed);
        }
    }

    /// Read the accumulated compute time in microseconds.
    pub(super) fn compute_us(&self) -> u64 {
        self.poll_compute_us.load(Ordering::Relaxed)
    }
}

/// Per-request metadata tracker.
///
/// Accumulates local compute time, downstream utilization, and subtree
/// compute cost throughout the request lifecycle. Builds `ResponseMeta`
/// for the outgoing response at finalization.
#[derive(Debug)]
pub(crate) struct RequestMetadataTracker {
    compute: ComputeTracker,
    max_child_downstream_util: Mutex<f32>,
    accumulated_child_compute_us: AtomicU64,
    accumulated_child_early_returns: AtomicU32,
    local_early_return: AtomicBool,
    // A single user-facing request that signals at multiple hops should still
    // count as one event; otherwise a 5-hop request that signals at every hop
    // looks like 5 separate failures. We track presence (AtomicBool), not a
    // running tally, so the ingress sees deadline_signal_count in {0, 1}.
    pub(super) child_deadline_signal: AtomicBool,
    local_deadline_signal: AtomicBool,
}

impl RequestMetadataTracker {
    pub(crate) fn new() -> Self {
        Self {
            compute: ComputeTracker::new(),
            max_child_downstream_util: Mutex::new(0.0),
            accumulated_child_compute_us: AtomicU64::new(0),
            accumulated_child_early_returns: AtomicU32::new(0),
            local_early_return: AtomicBool::new(false),
            child_deadline_signal: AtomicBool::new(false),
            local_deadline_signal: AtomicBool::new(false),
        }
    }

    /// Start tracking compute time for the current poll.
    pub(crate) fn start_poll(&self) {
        self.compute.start();
    }

    /// Stop tracking compute time after a poll.
    pub(crate) fn end_poll(&self) {
        self.compute.stop();
    }

    /// Extract response metadata from a child RPC response.
    ///
    /// Updates max downstream utilization and accumulated subtree compute cost.
    pub(crate) fn absorb_child_meta<T>(
        &self,
        response: &Result<Response<T>, Status>,
    ) -> ChildResponseInfo {
        let mut downstream_util = None;
        let mut accumulated_compute_us = None;

        if is_early_return_response(response) {
            // Child early-returned with Err(Status) — no response headers to
            // read, but we know at least 1 early return occurred.
            self.accumulated_child_early_returns
                .fetch_add(1, Ordering::Relaxed);
        } else if let Ok(resp) = response {
            if let Some(child_ctx_resp) = resp.get_masa_context() {
                if let Some(meta) = child_ctx_resp.response_meta() {
                    let mut max_util = self.max_child_downstream_util.lock().unwrap();
                    if meta.max_downstream_util > *max_util {
                        *max_util = meta.max_downstream_util;
                    }
                    downstream_util = Some(meta.max_downstream_util);
                    self.accumulated_child_compute_us
                        .fetch_add(meta.accumulated_compute_us, Ordering::Relaxed);
                    self.accumulated_child_early_returns
                        .fetch_add(meta.early_return_count, Ordering::Relaxed);
                    if meta.deadline_signal_count > 0 {
                        self.child_deadline_signal.store(true, Ordering::Relaxed);
                    }
                    accumulated_compute_us = Some(meta.accumulated_compute_us);
                }
            }
        }

        ChildResponseInfo {
            downstream_util,
            accumulated_compute_us,
        }
    }

    /// Mark this request as having triggered a local early return.
    pub(crate) fn mark_early_return(&self) {
        self.local_early_return.store(true, Ordering::Relaxed);
    }

    /// Mark this request as having tripped a soft deadline signal
    /// (`signal_slack`) - request continues, but the ingress AC sees the
    /// signal via `ResponseMeta.deadline_signal_count`.
    pub(crate) fn mark_deadline_signal(&self) {
        self.local_deadline_signal.store(true, Ordering::Relaxed);
    }

    /// True when this hop or any descendant tripped its local deadline under
    /// `signal_slack`. Used to gate latency-estimator updates so a
    /// signal-but-continue request's inflated wallclock doesn't poison the
    /// estimator.
    pub(crate) fn is_subtree_signaled(&self) -> bool {
        self.local_deadline_signal.load(Ordering::Relaxed)
            || self.child_deadline_signal.load(Ordering::Relaxed)
    }

    /// Build and set `ResponseMeta` on the outgoing context.
    pub(crate) fn inject_response_meta(&self, ctx: &mut Context) {
        let compute_time_us = self.compute.compute_us();
        let accumulated_compute_us =
            compute_time_us + self.accumulated_child_compute_us.load(Ordering::Relaxed);
        let utilization = tokio::task::current_utilization() as f32;
        let max_child_util = *self.max_child_downstream_util.lock().unwrap();
        let max_downstream_util = utilization.max(max_child_util);

        let local_er = if self.local_early_return.load(Ordering::Relaxed) {
            1
        } else {
            0
        };
        let early_return_count =
            local_er + self.accumulated_child_early_returns.load(Ordering::Relaxed);

        // Saturate at 1: a single ingress request signals at most once,
        // regardless of how many hops in its subtree tripped their local
        // deadline. The AC reads this as a boolean (`> 0`) anyway.
        let signaled = self.local_deadline_signal.load(Ordering::Relaxed)
            || self.child_deadline_signal.load(Ordering::Relaxed);
        let deadline_signal_count = if signaled { 1 } else { 0 };

        ctx.set_response_meta(ResponseMeta {
            compute_time_us,
            accumulated_compute_us,
            utilization,
            max_downstream_util,
            early_return_count,
            deadline_signal_count,
        });
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Helpers
// ═════════════════════════════════════════════════════════��════════════════

pub(crate) fn is_early_return_response<T>(response: &Result<Response<T>, Status>) -> bool {
    match response {
        Ok(_) => false,
        Err(status) => status.code() == Code::DeadlineExceeded,
    }
}

/// True when an Ok response carries a non-zero `deadline_signal_count` —
/// i.e., the request returned successfully but tripped its local deadline at
/// some hop under `signal_slack`. The wallclock for such a request is
/// inflated by signal-but-continue runtime, so the latency estimator should
/// skip these observations the same way it skips Err early-returns.
pub(crate) fn is_signaled_response<T>(response: &Result<Response<T>, Status>) -> bool {
    if let Ok(resp) = response {
        if let Some(ctx) = resp.get_masa_context() {
            if let Some(meta) = ctx.response_meta() {
                return meta.deadline_signal_count > 0;
            }
        }
    }
    false
}
