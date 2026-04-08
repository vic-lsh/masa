// Latency estimation state for Masa scheduling policies.
//
// Provides `LatencyEstimators` (shared server-level maps with domain-specific
// query/track facade), `EstimationTracker` (per-request estimation lifecycle),
// `ChildRPCTracker` (per-child-RPC timing), and `RequestMetadataTracker`
// (per-request response metadata assembly).
//
// Used by `PredAdmissionLayer` for deadline tightening, dynamic
// reprioritization, and admission control.

use std::fmt;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use masa_core::{Context, LatencyEstimator, ResponseMeta};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::latency_map::{LatencyMap, MethodKey, ParentToChildKey, RootToLocalKey};
use crate::context_ext::MasaResponseExt;
use crate::registry::MethodId;
use crate::MethodRegistry;

// ══════════════════════════════════════════════════════════════════════════
// Remaining-wallclock estimate bundle
// ══════════════════════════════════════════════════════════════════════════

/// Three estimate flavors for after-child wall-clock time (parent's post-child work duration).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AfterChildEstimates {
    /// Full estimate (mean + k*stddev, capped at time_left).
    pub full: u64,
    /// Mean-only estimate (k=0, capped at time_left).
    pub mean: u64,
    /// Floor estimate (spike-resistant, capped at time_left).
    pub floor: u64,
}

// ═════════════════════════════════════════════════════════════��════════════
// Server-level shared estimation state
// ══════════════════════════════════════════════════════════════════════════

/// Server-level estimation state (shared across requests on a service).
///
/// All maps and the print counter live behind one [`Arc`] so trackers clone a single handle.
#[derive(Debug)]
pub(crate) struct LatencyEstimatorsInner<E: LatencyEstimator + Default + 'static> {
    /// Remaining wall-clock time after each child RPC completes (parent→child key).
    after_child_wallclock: LatencyMap<ParentToChildKey, E>,
    /// Wall-clock duration of child RPC calls (parent→child key).
    child_wallclock: LatencyMap<ParentToChildKey, E>,
    /// Accumulated CPU compute time per request subtree (root API key).
    subtree_compute: LatencyMap<MethodKey, E>,
    /// Total wall-clock latency per method (root API type → local method key).
    method_wallclock: LatencyMap<RootToLocalKey, E>,
    /// Counter for periodic logging.
    print_counter: AtomicUsize,
}

#[cfg(test)]
impl<E: LatencyEstimator + Default + 'static> LatencyEstimatorsInner<E> {
    /// Direct access to the child-wallclock map for testing with custom estimators.
    pub(crate) fn child_wallclock_map(&self) -> &LatencyMap<ParentToChildKey, E> {
        &self.child_wallclock
    }
}

/// Cheap clone: shares the same [`LatencyEstimatorsInner`] as other trackers on the service.
#[derive(Debug)]
pub(crate) struct LatencyEstimators<E: LatencyEstimator + Default + 'static>(
    Arc<LatencyEstimatorsInner<E>>,
);

impl<E: LatencyEstimator + Default + 'static> Clone for LatencyEstimators<E> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<E: LatencyEstimator + Default + 'static> Deref for LatencyEstimators<E> {
    type Target = LatencyEstimatorsInner<E>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<E: LatencyEstimator + Default + 'static> LatencyEstimators<E> {
    pub(crate) fn new() -> Self {
        let inner = Arc::new(LatencyEstimatorsInner {
            after_child_wallclock: LatencyMap::new(),
            child_wallclock: LatencyMap::new(),
            subtree_compute: LatencyMap::new(),
            method_wallclock: LatencyMap::new(),
            print_counter: AtomicUsize::new(0),
        });

        spawn_latency_estimators_stats_printer(Arc::clone(&inner));

        Self(inner)
    }

    // ── Estimate queries ───────────────────────────────────────────────

    /// Estimated wall-clock time from child RPC completion to parent end, capped at `cap`.
    /// Returns all three estimate flavors (full, mean, floor).
    pub(crate) fn est_after_child_wallclock(
        &self,
        key: ParentToChildKey,
        cap: u64,
    ) -> AfterChildEstimates {
        AfterChildEstimates {
            full: self
                .after_child_wallclock
                .get_estimate(key)
                .unwrap_or(0)
                .min(cap),
            mean: self
                .after_child_wallclock
                .get_mean_estimate(key)
                .unwrap_or(0)
                .min(cap),
            floor: self
                .after_child_wallclock
                .get_mean_floor_estimate(key)
                .unwrap_or(0)
                .min(cap),
        }
    }

    /// Estimated wall-clock duration of a child RPC call.
    pub(crate) fn est_child_wallclock(&self, key: ParentToChildKey) -> Option<u64> {
        self.child_wallclock.get_estimate(key)
    }

    /// Estimated accumulated CPU compute cost for a request subtree (root API key).
    #[cfg(feature = "ac_pred")]
    #[allow(dead_code)]
    pub(crate) fn est_subtree_compute(&self, key: MethodKey) -> Option<u64> {
        self.subtree_compute.get_estimate(key)
    }

    // ── Tracking ───────────────────────────────────────────────────────

    /// Track wall-clock time from child RPC completion to parent request end.
    pub(crate) fn track_after_child_wallclock(&self, key: ParentToChildKey, duration_us: u64) {
        self.after_child_wallclock.track(key, duration_us);
    }

    /// Track wall-clock duration of a child RPC call.
    pub(crate) fn track_child_wallclock(&self, key: ParentToChildKey, duration_us: u64) {
        self.child_wallclock.track(key, duration_us);
    }

    /// Track total wall-clock latency for a (root API, local method) pair.
    pub(crate) fn track_method_wallclock(&self, key: RootToLocalKey, duration_us: u64) {
        self.method_wallclock.track(key, duration_us);
    }

    /// Track accumulated CPU compute cost for a request subtree (root API key).
    #[cfg(feature = "ac_pred")]
    pub(crate) fn track_subtree_compute(&self, key: MethodKey, cost_us: u64) {
        self.subtree_compute.track(key, cost_us);
    }

    /// Inject 0 into after-child-wallclock estimates when a child early-returns.
    /// Creates negative feedback to prevent frozen high estimates.
    pub(crate) fn track_er_feedback(&self, key: ParentToChildKey) {
        self.after_child_wallclock.track(key, 0);
    }

    // ── Logging ────────────────────────────────────────────────────────

    /// Periodic logging of latency estimates for a parent→child pair.
    pub(crate) fn log_estimates(&self, key: &ParentToChildKey, remaining: &AfterChildEstimates) {
        if self.print_counter.fetch_add(1, Ordering::Relaxed) % 5000 == 0 {
            let est_child = self.est_child_wallclock(*key).unwrap_or(0);
            log::info!(
                "LAT_EST: p=>c: {}, est_child: {}, est_rem: {}, est_rem_mean: {}, est_rem_floor: {}",
                key,
                est_child,
                remaining.full,
                remaining.mean,
                remaining.floor,
            );
        }
    }
}

fn spawn_latency_estimators_stats_printer<E>(shared: Arc<LatencyEstimatorsInner<E>>)
where
    E: LatencyEstimator + Default + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                log_pair_map_if_non_empty(
                    "Est After-Child Wallclock",
                    &shared.after_child_wallclock,
                );
                log_pair_map_if_non_empty("Est Child Wallclock", &shared.child_wallclock);
                log_method_map_if_non_empty("Est Subtree Compute", &shared.subtree_compute);
                log_pair_map_if_non_empty("Est Method Wallclock", &shared.method_wallclock);
            }
        });
    }
}

fn log_pair_map_if_non_empty<K, E>(label: &'static str, map: &LatencyMap<K, E>)
where
    K: Copy + Eq + Hash + fmt::Display + 'static,
    E: LatencyEstimator + Default + 'static,
{
    if map.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    map.for_each(|key, distribution| {
        if distribution.can_estimate() {
            parts.push(format!("{}: {} us", key, distribution.estimate()));
        } else {
            parts.push(format!("{}: (no estimate)", key));
        }
    });
    log::info!("{}: {}", label, parts.join(", "));
}

fn log_method_map_if_non_empty<E: LatencyEstimator + Default + 'static>(
    label: &'static str,
    map: &LatencyMap<MethodKey, E>,
) {
    if map.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    map.for_each(|key, distribution| {
        let name = super::latency_map::format_method_name(key.0);
        if distribution.can_estimate() {
            parts.push(format!("{}: {} us", name, distribution.estimate()));
        } else {
            parts.push(format!("{}: (no estimate)", name));
        }
    });
    log::info!("{}: {}", label, parts.join(", "));
}

// ══════════════════════════════════════════════════════════════════════════
// Per-request estimation lifecycle
// ══════════════════════════════════════════════════════════════════════════

/// Per-request estimation state: tracks observations and queries estimates.
#[derive(Debug)]
pub(crate) struct EstimationTracker<E: LatencyEstimator + Default + 'static> {
    pub(crate) resolved_method_id: MethodId,
    pub(crate) root_method_id: Option<MethodId>,
    pub(crate) est: LatencyEstimators<E>,
    child_end_times: Mutex<Vec<(ParentToChildKey, Instant)>>,
    request_start: Instant,
}

impl<E: LatencyEstimator + Default + 'static> EstimationTracker<E> {
    pub(crate) fn new(
        resolved_method_id: MethodId,
        root_method_id: Option<MethodId>,
        est: LatencyEstimators<E>,
    ) -> Self {
        Self {
            resolved_method_id,
            root_method_id,
            est,
            child_end_times: Mutex::new(Vec::new()),
            request_start: Instant::now(),
        }
    }

    /// Create a child RPC tracker for the given child method.
    pub(crate) fn begin_child(&self, child_method: &CowGrpcMethod) -> ChildRPCTracker {
        let child_id = MethodRegistry::global().get_or_register(child_method.clone());
        let key =
            ParentToChildKey::parent_rpc_method(self.resolved_method_id).child_rpc_method(child_id);
        ChildRPCTracker::new(key)
    }

    /// Record a completed child RPC: track child wallclock on success, or inject
    /// ER feedback on deadline-exceeded.
    pub(crate) fn record_child_complete<T>(
        &self,
        tracker: &ChildRPCTracker,
        response: &Result<Response<T>, Status>,
    ) {
        if is_early_return_response(response) {
            self.est.track_er_feedback(tracker.key);
        } else {
            self.est
                .track_child_wallclock(tracker.key, tracker.elapsed_us());
            self.child_end_times
                .lock()
                .unwrap()
                .push((tracker.key, Instant::now()));
        }
    }

    /// Flush deferred observations at request finalization.
    ///
    /// Tracks remaining-wallclock (time from child end to parent end) and
    /// method-wallclock (total request duration).
    pub(crate) fn flush(&self) {
        let parent_end = Instant::now();

        let child_end_times = std::mem::take(&mut *self.child_end_times.lock().unwrap());
        for (key, child_end) in child_end_times {
            self.est.track_after_child_wallclock(
                key,
                parent_end.duration_since(child_end).as_micros() as u64,
            );
        }

        if let Some(key) = self.method_wallclock_key() {
            let total_wall_clock = self.request_start.elapsed().as_micros() as u64;
            self.est.track_method_wallclock(key, total_wall_clock);
        }
    }

    /// Compound key for tracking into `method_wallclock`.
    fn method_wallclock_key(&self) -> Option<RootToLocalKey> {
        let root = self.root_method_id?;
        Some(RootToLocalKey::root_rpc_method(root).local_rpc_method(self.resolved_method_id))
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Per-child-RPC tracker
// ══════════════════════════════════════════════════════════════════════════

/// Per-child-RPC timing state.
///
/// Created by `EstimationTracker::begin_child` when the child method and
/// parent→child relationship are known.
#[derive(Debug, Clone)]
pub(crate) struct ChildRPCTracker {
    pub key: ParentToChildKey,
    start_time: Instant,
}

impl ChildRPCTracker {
    fn new(key: ParentToChildKey) -> Self {
        Self {
            key,
            start_time: Instant::now(),
        }
    }

    /// Wall-clock microseconds elapsed since this child RPC started.
    pub(crate) fn elapsed_us(&self) -> u64 {
        self.start_time.elapsed().as_micros() as u64
    }
}

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
    fn new() -> Self {
        Self {
            poll_compute_us: AtomicU64::new(0),
            poll_start: Mutex::new(None),
        }
    }

    /// Start tracking compute time for the current poll.
    fn start(&self) {
        *self.poll_start.lock().unwrap() = Some(Instant::now());
    }

    /// Stop tracking compute time and accumulate elapsed time.
    fn stop(&self) {
        if let Some(start) = self.poll_start.lock().unwrap().take() {
            let elapsed_us = start.elapsed().as_micros() as u64;
            self.poll_compute_us
                .fetch_add(elapsed_us, Ordering::Relaxed);
        }
    }

    /// Read the accumulated compute time in microseconds.
    fn compute_us(&self) -> u64 {
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
}

impl RequestMetadataTracker {
    pub(crate) fn new() -> Self {
        Self {
            compute: ComputeTracker::new(),
            max_child_downstream_util: Mutex::new(0.0),
            accumulated_child_compute_us: AtomicU64::new(0),
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

        if let Ok(resp) = response {
            if let Some(child_ctx_resp) = resp.get_masa_context() {
                if let Some(meta) = child_ctx_resp.response_meta() {
                    let mut max_util = self.max_child_downstream_util.lock().unwrap();
                    if meta.max_downstream_util > *max_util {
                        *max_util = meta.max_downstream_util;
                    }
                    downstream_util = Some(meta.max_downstream_util);
                    self.accumulated_child_compute_us
                        .fetch_add(meta.accumulated_compute_us, Ordering::Relaxed);
                    accumulated_compute_us = Some(meta.accumulated_compute_us);
                }
            }
        }

        ChildResponseInfo {
            downstream_util,
            accumulated_compute_us,
        }
    }

    /// Build and set `ResponseMeta` on the outgoing context.
    pub(crate) fn inject_response_meta(&self, ctx: &mut Context) {
        let compute_time_us = self.compute.compute_us();
        let accumulated_compute_us =
            compute_time_us + self.accumulated_child_compute_us.load(Ordering::Relaxed);
        let utilization = tokio::task::current_utilization() as f32;
        let max_child_util = *self.max_child_downstream_util.lock().unwrap();
        let max_downstream_util = utilization.max(max_child_util);

        ctx.set_response_meta(ResponseMeta {
            compute_time_us,
            accumulated_compute_us,
            utilization,
            max_downstream_util,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_tracker_accumulates_across_polls() {
        let tracker = ComputeTracker::new();
        assert_eq!(tracker.compute_us(), 0);

        tracker.start();
        std::thread::sleep(std::time::Duration::from_millis(1));
        tracker.stop();

        assert!(tracker.compute_us() > 0);

        let first = tracker.compute_us();
        tracker.start();
        std::thread::sleep(std::time::Duration::from_millis(1));
        tracker.stop();

        assert!(tracker.compute_us() > first);
    }

    #[test]
    fn compute_tracker_stop_without_start_is_noop() {
        let tracker = ComputeTracker::new();
        tracker.stop();
        assert_eq!(tracker.compute_us(), 0);
    }
}
