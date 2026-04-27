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
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

use masa_core::{Context, LatencyEstimator, ResponseMeta};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::fanout::{recover_groups, ChildRecord, FanoutPatternTable};
use super::latency_map::{LatencyMap, MethodKey, ParentToChildKey, RootToLocalKey};
use crate::context_ext::MasaResponseExt;
use crate::registry::MethodId;
use crate::MethodRegistry;

// ══════════════════════════════════════════════════════════════════════════
// Remaining-wallclock estimate bundle
// ══════════════════════════════════════════════════════════════════════════

/// Three estimate flavors for after-child wall-clock time (parent's post-child work duration).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AfterChildEstimates {
    /// Full estimate (mean + k*stddev, capped at time_left).
    pub full: u64,
    /// Mean-only estimate (k=0, capped at time_left).
    pub mean: u64,
    /// Floor estimate (spike-resistant, capped at time_left).
    pub floor: u64,
}

/// Runtime toggle for fanout-aware after-child estimation.
///
/// Reads `MASA_FANOUT_AWARE` once on first access; default is `true` (new
/// behavior). Set `MASA_FANOUT_AWARE=0` (or `false`/`off`/`no`) to use the
/// legacy per-edge `(parent, child)` map for both reads and writes — useful
/// for A/B testing the fix on the same binary.
pub(crate) fn fanout_aware_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let val = match std::env::var("MASA_FANOUT_AWARE") {
            Ok(v) => {
                let v = v.trim().to_ascii_lowercase();
                !matches!(v.as_str(), "0" | "false" | "off" | "no")
            }
            Err(_) => true,
        };
        log::info!("MASA_FANOUT_AWARE = {}", val);
        val
    })
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
    /// Used by the admission-control feasibility check; now updated with the
    /// fanout-corrected post-join time (`parent_end - max(group.end)`) so
    /// siblings on the critical path do not pollute it.
    after_child_wallclock: LatencyMap<ParentToChildKey, E>,
    /// Wall-clock duration of child RPC calls (parent→child key).
    child_wallclock: LatencyMap<ParentToChildKey, E>,
    /// Accumulated CPU compute time per request subtree (root API key).
    subtree_compute: LatencyMap<MethodKey, E>,
    /// Total wall-clock latency per method (root API type → local method key).
    method_wallclock: LatencyMap<RootToLocalKey, E>,
    /// Fanout-group post-join estimates, keyed on (parent_method, sorted
    /// multiset of sibling child_methods). Read at child-issue time for
    /// deadline tightening.
    fanout_patterns: FanoutPatternTable<E>,
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
            fanout_patterns: FanoutPatternTable::new(),
            print_counter: AtomicUsize::new(0),
        });

        spawn_latency_estimators_stats_printer(Arc::clone(&inner));

        Self(inner)
    }

    // ── Estimate queries ───────────────────────────────────────────────

    /// Estimated wall-clock time from child RPC completion to parent end, capped at `cap`.
    /// Returns all three estimate flavors (full, mean, floor).
    /// Used by the admission feasibility check; the estimation/scheduling
    /// path uses the fanout-aware variant below.
    #[allow(dead_code)]
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

    /// Lookup an after-child wall-clock estimate for the child being
    /// issued. When the runtime toggle [`fanout_aware_enabled`] is true
    /// (default), use the fanout-group pattern table keyed on
    /// `base_signature`; otherwise fall back to the legacy per-edge map
    /// keyed on `(parent, new_child_id)`.
    pub(crate) fn est_after_child_wallclock_for_group(
        &self,
        parent: MethodId,
        base_signature: &[MethodId],
        new_child_id: MethodId,
        cap: u64,
    ) -> AfterChildEstimates {
        if fanout_aware_enabled() {
            self.fanout_patterns
                .lookup_estimate(parent, base_signature, cap)
        } else {
            let key = ParentToChildKey::parent_rpc_method(parent).child_rpc_method(new_child_id);
            self.est_after_child_wallclock(key, cap)
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

    /// Track one observed fanout group's post-join time.
    pub(crate) fn track_fanout_group(
        &self,
        parent: MethodId,
        signature: Vec<MethodId>,
        post_join_us: u64,
    ) {
        self.fanout_patterns.update(parent, signature, post_join_us);
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

    /// No-op when a child early-returns: we do not update after-child-wallclock.
    ///
    /// Previously this injected 0, which prevented frozen-high estimates but caused
    /// burst-clear-rebound oscillation: a burst of ERs (alpha_down=0.2 per obs) would
    /// collapse the estimate toward 0, silencing abort_slack until the queue rebuilt.
    /// Letting the estimate stale at its last real observation is safer — it preserves
    /// abort_slack pressure during the burst, and the estimate self-corrects once
    /// successful completions resume.
    #[allow(unused_variables)]
    pub(crate) fn track_er_feedback(&self, key: ParentToChildKey) {}

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
                log_fanout_patterns_if_non_empty(&shared.fanout_patterns);
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

fn log_fanout_patterns_if_non_empty<E: LatencyEstimator + Default + 'static>(
    table: &FanoutPatternTable<E>,
) {
    if table.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    table.for_each_pattern(|parent, signature, est, count| {
        let parent_name = super::latency_map::format_method_name(parent);
        let sig_str = signature
            .iter()
            .map(|id| super::latency_map::format_method_name(*id))
            .collect::<Vec<_>>()
            .join(",");
        let est_str = if est.can_estimate() {
            format!("{} us", est.estimate())
        } else {
            "(no estimate)".to_string()
        };
        parts.push(format!(
            "{}|[{}] n={}: {}",
            parent_name, sig_str, count, est_str
        ));
    });
    log::info!("Est Fanout Groups: {}", parts.join(" ;; "));
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
    /// All child RPCs issued under this handler invocation, in issue order.
    /// Used at handler exit to recover fanout groups via interval-overlap.
    children: Mutex<Vec<ChildRecord>>,
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
            children: Mutex::new(Vec::new()),
            request_start: Instant::now(),
        }
    }

    /// Create a child RPC tracker for the given child method.
    ///
    /// Records the child's start and computes a `base_signature`: the sorted
    /// multiset of still-open earlier children plus this child. That set is
    /// guaranteed to be a multiset-subset of the eventual fanout group, so
    /// it is the issue-time lower bound used to look up a group estimate.
    pub(crate) fn begin_child(&self, child_method: &CowGrpcMethod) -> ChildRPCTracker {
        let child_id = MethodRegistry::global().get_or_register(child_method.clone());
        let key =
            ParentToChildKey::parent_rpc_method(self.resolved_method_id).child_rpc_method(child_id);
        let now = Instant::now();
        let mut children = self.children.lock().unwrap();
        let mut base_signature: Vec<MethodId> = children
            .iter()
            .filter(|c| c.end.is_none())
            .map(|c| c.child_id)
            .collect();
        base_signature.push(child_id);
        base_signature.sort();
        let index = children.len();
        children.push(ChildRecord {
            child_id,
            start: now,
            end: None,
        });
        ChildRPCTracker::new(key, child_id, index, base_signature, now)
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
            // Leave the ChildRecord with end=None so it is excluded from
            // group recovery — early-returned children carry no useful
            // post-join signal.
        } else {
            self.est
                .track_child_wallclock(tracker.key, tracker.elapsed_us());
            let now = Instant::now();
            let mut children = self.children.lock().unwrap();
            if let Some(record) = children.get_mut(tracker.child_index) {
                record.end = Some(now);
            }
        }
    }

    /// Flush deferred observations at request finalization.
    ///
    /// When fanout-aware estimation is enabled, recovers fanout groups from
    /// observed intervals, updates the fanout pattern table, and mirrors
    /// the corrected post-join time into the per-edge map so admission
    /// feasibility checks see the same value. When disabled, reproduces
    /// the legacy per-child delta `parent_end - child.end` to keep the A/B
    /// test honest.
    pub(crate) fn flush(&self) {
        let parent_end = Instant::now();

        let children = std::mem::take(&mut *self.children.lock().unwrap());
        if fanout_aware_enabled() {
            let groups = recover_groups(&children, parent_end);
            for (signature, post_join_us) in &groups {
                self.est.track_fanout_group(
                    self.resolved_method_id,
                    signature.clone(),
                    *post_join_us,
                );
                for child_id in signature {
                    let key = ParentToChildKey::parent_rpc_method(self.resolved_method_id)
                        .child_rpc_method(*child_id);
                    self.est.track_after_child_wallclock(key, *post_join_us);
                }
            }
        } else {
            for c in &children {
                let Some(end) = c.end else {
                    continue;
                };
                let key = ParentToChildKey::parent_rpc_method(self.resolved_method_id)
                    .child_rpc_method(c.child_id);
                let after = parent_end.saturating_duration_since(end).as_micros() as u64;
                self.est.track_after_child_wallclock(key, after);
            }
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
    /// Index into the parent EstimationTracker's `children` vec.
    pub(crate) child_index: usize,
    /// Sorted multiset of still-open earlier children + this child at issue
    /// time. Lower bound on the eventual fanout-group signature; used by
    /// the estimation layer to look up the matching group's EMA.
    pub(crate) base_signature: Vec<MethodId>,
    #[allow(dead_code)]
    pub(crate) child_id: MethodId,
    start_time: Instant,
}

impl ChildRPCTracker {
    fn new(
        key: ParentToChildKey,
        child_id: MethodId,
        child_index: usize,
        base_signature: Vec<MethodId>,
        start_time: Instant,
    ) -> Self {
        Self {
            key,
            child_index,
            base_signature,
            child_id,
            start_time,
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
    accumulated_child_early_returns: AtomicU32,
    local_early_return: AtomicBool,
}

impl RequestMetadataTracker {
    pub(crate) fn new() -> Self {
        Self {
            compute: ComputeTracker::new(),
            max_child_downstream_util: Mutex::new(0.0),
            accumulated_child_compute_us: AtomicU64::new(0),
            accumulated_child_early_returns: AtomicU32::new(0),
            local_early_return: AtomicBool::new(false),
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

        ctx.set_response_meta(ResponseMeta {
            compute_time_us,
            accumulated_compute_us,
            utilization,
            max_downstream_util,
            early_return_count,
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
