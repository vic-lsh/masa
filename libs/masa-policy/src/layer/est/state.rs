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

use super::fanout::{
    recover_path_groups, ChildRecord, FanoutPatternTable, PathPrefix, PatternScope,
};
use super::latency_map::{LatencyMap, MethodKey, ParentToChildKey, RootToLocalKey};
use crate::context_ext::MasaResponseExt;
use crate::policy_params::PolicyParams;
use crate::registry::MethodId;
use crate::MethodRegistry;

// ══════════════════════════════════════════════════════════════════════════
// Wallclock-time decay for stale estimates
// ══════════════════════════════════════════════════════════════════════════

/// Wallclock-time decay constant (microseconds) for stale EMA estimates.
/// At age = TAU_DECAY_US, the contribution is multiplied by 1/e ≈ 0.37.
/// Picked at 5 s so that a sustained admission lockout breaks within tens of
/// seconds even without fresh observations.
pub(crate) const TAU_DECAY_US: f64 = 5_000_000.0;

/// Skip the `exp()` call when the most recent observation is younger than this.
/// At ages well below TAU_DECAY_US the decay factor is ≈ 1.0 anyway; avoiding
/// the transcendental keeps the hot path cheap.
pub(crate) const DECAY_THRESHOLD_US: u64 = 200_000;

/// `exp(-x)` for `x ≥ 0`, with an early-out for arguments large enough that
/// the result rounds to 0. For typical arguments this is just `f64::exp(-x)`,
/// ~10 ns on x86.
#[inline(always)]
pub(crate) fn fast_exp_neg(x: f64) -> f64 {
    debug_assert!(x >= 0.0);
    if x >= 50.0 {
        return 0.0;
    }
    (-x).exp()
}

/// Decay factor for an estimate whose most recent observation is `last_obs_us`
/// (microseconds since epoch), evaluated at `now_us`. Returns 1.0 for fresh
/// observations and shrinks toward 0 as the age grows past `TAU_DECAY_US`.
#[inline]
pub(crate) fn decay_factor(now_us: u64, last_obs_us: u64) -> f64 {
    let age = now_us.saturating_sub(last_obs_us);
    if age <= DECAY_THRESHOLD_US {
        1.0
    } else {
        fast_exp_neg(age as f64 / TAU_DECAY_US)
    }
}

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

impl AfterChildEstimates {
    fn is_empty(self) -> bool {
        self.full == 0 && self.mean == 0 && self.floor == 0
    }

    fn merge_with_legacy_priority(self, legacy: Self, deadline_legacy_fraction: f64) -> Self {
        if self.is_empty() {
            return legacy;
        }
        if legacy.is_empty() {
            return self;
        }
        Self {
            // `full` is used as the soft scheduling priority signal. Keep
            // the legacy per-edge value so fanout correction does not
            // under-prioritize children that still have substantial service
            // time ahead of them.
            full: legacy.full,
            // `mean`/`floor` feed hard deadline tightening. There fanout is
            // allowed to remove sibling-wait noise, but never tighten beyond
            // the legacy estimate.
            mean: blend_toward_legacy(
                self.mean.min(legacy.mean),
                legacy.mean,
                deadline_legacy_fraction,
            ),
            floor: blend_toward_legacy(
                self.floor.min(legacy.floor),
                legacy.floor,
                deadline_legacy_fraction,
            ),
        }
    }
}

fn blend_toward_legacy(value: u64, legacy: u64, fraction: f64) -> u64 {
    if value >= legacy {
        return legacy;
    }
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= f64::EPSILON {
        return value;
    }
    if (1.0 - fraction) <= f64::EPSILON {
        return legacy;
    }
    value + ((legacy - value) as f64 * fraction).round() as u64
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

fn fanout_enabled_for_parent(root: MethodId, parent: MethodId) -> bool {
    if !fanout_aware_enabled() {
        return false;
    }
    let params = &PolicyParams::global().pred;
    !params.fanout_root_only || root == parent
}

/// Runtime toggle for verbose estimator table dumps.
///
/// The fanout table can contain thousands of method-level signatures on mssim
/// traces. Walking and formatting that table every few seconds is useful while
/// debugging, but it is far too expensive for performance experiments, so keep
/// it opt-in.
fn estimator_stats_logging_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let val = match std::env::var("MASA_ESTIMATOR_STATS_LOG") {
            Ok(v) => {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "on" | "yes")
            }
            Err(_) => false,
        };
        log::info!("MASA_ESTIMATOR_STATS_LOG = {}", val);
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
    /// Coarser fanout-group estimates keyed on child service multisets.
    /// This is a low-cardinality fallback for traces whose exact child method
    /// signatures are too sparse to estimate reliably.
    service_fanout_patterns: FanoutPatternTable<E>,
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
            service_fanout_patterns: FanoutPatternTable::new(),
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
    /// (default), use the fanout-group pattern table keyed on the observable
    /// path prefix plus `base_signature`; otherwise fall back to the legacy
    /// per-edge map keyed on `(parent, new_child_id)`.
    pub(crate) fn est_after_child_wallclock_for_group(
        &self,
        root: MethodId,
        parent: MethodId,
        path_prefix: PathPrefix,
        base_signature: &[MethodId],
        service_path_prefix: PathPrefix,
        base_service_signature: &[MethodId],
        new_child_id: MethodId,
        cap: u64,
    ) -> AfterChildEstimates {
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(parent)
            .child_rpc_method(new_child_id);
        let legacy = self.est_after_child_wallclock(key, cap);
        if !fanout_enabled_for_parent(root, parent) || base_signature.len() <= 1 {
            return legacy;
        }

        let min_samples = PolicyParams::global().pred.fanout_min_samples;
        let fanout = self.fanout_patterns.lookup_estimate(
            root,
            parent,
            path_prefix,
            base_signature,
            min_samples,
            cap,
        );
        let fanout = if fanout.is_empty() {
            self.service_fanout_patterns.lookup_estimate(
                root,
                parent,
                service_path_prefix,
                base_service_signature,
                min_samples,
                cap,
            )
        } else {
            fanout
        };

        fanout.merge_with_legacy_priority(
            legacy,
            PolicyParams::global().pred.fanout_deadline_legacy_fraction,
        )
    }

    /// Estimated wall-clock duration of a child RPC call.
    pub(crate) fn est_child_wallclock(&self, key: ParentToChildKey) -> Option<u64> {
        self.child_wallclock.get_estimate(key)
    }

    /// Single-lookup pack of (mean, floor, last_observation_us) for the child
    /// wall-clock estimator. Used by the time-decay-aware BCF feasibility check.
    #[allow(dead_code)]
    pub(crate) fn child_wallclock_pack(
        &self,
        key: ParentToChildKey,
    ) -> Option<super::latency_map::EstimatesPack> {
        self.child_wallclock.get_estimates_pack(key)
    }

    /// Wall-clock timestamp of the most recent observation for the after-child
    /// remaining-wall-clock estimator on a given (root, parent, child) edge.
    /// Used by the BCF check to gauge how stale `remaining.floor` is.
    #[allow(dead_code)]
    pub(crate) fn after_child_wallclock_last_obs(&self, key: ParentToChildKey) -> u64 {
        self.after_child_wallclock
            .get_estimates_pack(key)
            .map(|p| p.last_observation_us)
            .unwrap_or(0)
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
        root: MethodId,
        parent: MethodId,
        path_prefix: PathPrefix,
        signature: Vec<MethodId>,
        service_path_prefix: PathPrefix,
        service_signature: Vec<MethodId>,
        post_join_us: u64,
    ) {
        self.fanout_patterns.update(
            root,
            parent,
            PatternScope::Path(path_prefix),
            signature.clone(),
            post_join_us,
        );
        self.fanout_patterns.update(
            root,
            parent,
            PatternScope::Aggregate,
            signature,
            post_join_us,
        );
        self.service_fanout_patterns.update(
            root,
            parent,
            PatternScope::Path(service_path_prefix),
            service_signature.clone(),
            post_join_us,
        );
        self.service_fanout_patterns.update(
            root,
            parent,
            PatternScope::Aggregate,
            service_signature,
            post_join_us,
        );
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
    if !estimator_stats_logging_enabled() {
        return;
    }

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
                log_fanout_patterns_if_non_empty("Est Fanout Groups", &shared.fanout_patterns);
                log_fanout_patterns_if_non_empty(
                    "Est Service Fanout Groups",
                    &shared.service_fanout_patterns,
                );
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
    label: &'static str,
    table: &FanoutPatternTable<E>,
) {
    if table.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    table.for_each_pattern(|root, parent, scope, signature, est, count| {
        let root_name = super::latency_map::format_method_name(root);
        let parent_name = super::latency_map::format_method_name(parent);
        let scope_name = match scope {
            PatternScope::Aggregate => "agg".to_string(),
            PatternScope::Path(prefix) => format!("p={}", prefix.label()),
        };
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
            "[{}]{}|{}|[{}] n={}: {}",
            root_name, parent_name, scope_name, sig_str, count, est_str
        ));
    });
    log::info!("{}: {}", label, parts.join(" ;; "));
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

/// Per-parent invocation state for deriving non-invasive fanout path prefixes.
#[derive(Debug, Default)]
struct FanoutInvocationState {
    /// All child RPCs issued under this handler invocation, in issue order.
    children: Vec<ChildRecord>,
    /// Prefix formed by completed fanout groups before the current open group.
    path_prefix: PathPrefix,
    /// Service-shape version of `path_prefix`.
    service_path_prefix: PathPrefix,
    /// Number of leading child records already folded into `path_prefix`.
    incorporated_child_count: usize,
}

impl FanoutInvocationState {
    fn refresh_path_prefix_before_open_group(&mut self, now: Instant) {
        if self.incorporated_child_count >= self.children.len() {
            return;
        }

        let pending = &self.children[self.incorporated_child_count..];
        let terminal_prefix_len = pending
            .iter()
            .position(|c| !c.terminal)
            .unwrap_or(pending.len());
        if terminal_prefix_len == 0 {
            return;
        }

        for group in recover_path_groups(&pending[..terminal_prefix_len], now) {
            self.path_prefix = self.path_prefix.append_signature(&group.signature);
            self.service_path_prefix = self
                .service_path_prefix
                .append_signature(&group.service_signature);
        }
        self.incorporated_child_count += terminal_prefix_len;
    }
}

/// Per-request estimation state: tracks observations and queries estimates.
#[derive(Debug)]
pub(crate) struct EstimationTracker<E: LatencyEstimator + Default + 'static> {
    pub(crate) resolved_method_id: MethodId,
    pub(crate) root_method_id: Option<MethodId>,
    pub(crate) est: LatencyEstimators<E>,
    /// Child RPC observations and the observable path prefix for the current
    /// in-flight fanout group.
    fanout: Mutex<FanoutInvocationState>,
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
            fanout: Mutex::new(FanoutInvocationState::default()),
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
        let (child_id, child_service_id) =
            MethodRegistry::global().get_or_register_with_service(child_method.clone());
        let root = self.root_or_self();
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(self.resolved_method_id)
            .child_rpc_method(child_id);
        let now = Instant::now();
        let mut fanout = self.fanout.lock().unwrap();
        let (path_prefix, base_signature, service_path_prefix, base_service_signature) =
            if fanout_enabled_for_parent(root, self.resolved_method_id) {
                let has_open_siblings = fanout.children.iter().any(|c| !c.terminal);
                if has_open_siblings {
                    fanout.refresh_path_prefix_before_open_group(now);
                }
                let path_prefix = fanout.path_prefix;
                let service_path_prefix = fanout.service_path_prefix;
                let mut base_signature: Vec<MethodId> = fanout
                    .children
                    .iter()
                    .filter(|c| !c.terminal)
                    .map(|c| c.child_id)
                    .collect();
                base_signature.push(child_id);
                base_signature.sort();
                let mut base_service_signature: Vec<MethodId> = fanout
                    .children
                    .iter()
                    .filter(|c| !c.terminal)
                    .map(|c| c.child_service_id)
                    .collect();
                base_service_signature.push(child_service_id);
                base_service_signature.sort();
                (
                    path_prefix,
                    base_signature,
                    service_path_prefix,
                    base_service_signature,
                )
            } else {
                (
                    PathPrefix::root(),
                    vec![child_id],
                    PathPrefix::root(),
                    vec![child_service_id],
                )
            };
        let index = fanout.children.len();
        fanout.children.push(ChildRecord {
            child_id,
            child_service_id,
            start: now,
            end: None,
            terminal: false,
        });
        ChildRPCTracker::new(
            key,
            child_id,
            index,
            path_prefix,
            base_signature,
            service_path_prefix,
            base_service_signature,
            now,
        )
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
            let mut fanout = self.fanout.lock().unwrap();
            if let Some(record) = fanout.children.get_mut(tracker.child_index) {
                // Leave end=None so it is excluded from group recovery, but
                // mark it terminal so later groups can advance the path prefix.
                record.terminal = true;
            }
        } else if is_signaled_response(response) {
            // Child returned Ok but signaled at some hop in its subtree under
            // signal_slack — its wallclock includes signal-but-continue
            // runtime. Skip track_child_wallclock so the estimator doesn't
            // over-tighten this parent->child key. Still mark the fanout
            // record end so after_child_wallclock is recorded normally if
            // this parent's flush runs.
            let mut fanout = self.fanout.lock().unwrap();
            if let Some(record) = fanout.children.get_mut(tracker.child_index) {
                record.end = Some(Instant::now());
                record.terminal = true;
            }
        } else {
            self.est
                .track_child_wallclock(tracker.key, tracker.elapsed_us());
            let now = Instant::now();
            let mut fanout = self.fanout.lock().unwrap();
            if let Some(record) = fanout.children.get_mut(tracker.child_index) {
                record.end = Some(now);
                record.terminal = true;
            }
        }
    }

    /// Flush deferred observations at request finalization.
    ///
    /// Always refreshes the legacy per-child delta `parent_end - child.end`
    /// for admission checks and as a fanout fallback. When fanout-aware
    /// estimation is enabled, also recovers fanout groups from observed
    /// intervals and updates the fanout pattern table. Lookup takes the
    /// smaller of the fanout and legacy estimates, so fanout correction can
    /// remove sibling-wait noise without making hard deadlines tighter than
    /// the legacy estimator.
    pub(crate) fn flush(&self) {
        let parent_end = Instant::now();

        let root = self.root_or_self();
        let fanout = std::mem::take(&mut *self.fanout.lock().unwrap());
        let children = fanout.children;
        for c in &children {
            let Some(end) = c.end else {
                continue;
            };
            let key = ParentToChildKey::root_rpc_method(root)
                .parent_rpc_method(self.resolved_method_id)
                .child_rpc_method(c.child_id);
            let after = parent_end.saturating_duration_since(end).as_micros() as u64;
            self.est.track_after_child_wallclock(key, after);
        }

        if fanout_enabled_for_parent(root, self.resolved_method_id) {
            let groups = recover_path_groups(&children, parent_end);
            for group in &groups {
                if group.signature.len() <= 1 {
                    continue;
                }
                self.est.track_fanout_group(
                    root,
                    self.resolved_method_id,
                    group.path_prefix,
                    group.signature.clone(),
                    group.service_path_prefix,
                    group.service_signature.clone(),
                    group.post_join_us,
                );
            }
        }

        if let Some(key) = self.method_wallclock_key() {
            let total_wall_clock = self.request_start.elapsed().as_micros() as u64;
            self.est.track_method_wallclock(key, total_wall_clock);
        }
    }

    /// Root API id, falling back to the resolved local method when the
    /// context did not carry one (ingress-only requests, tests).
    fn root_or_self(&self) -> MethodId {
        self.root_method_id.unwrap_or(self.resolved_method_id)
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
    /// Observable path prefix before this child's fanout group.
    pub(crate) path_prefix: PathPrefix,
    /// Sorted multiset of still-open earlier children + this child at issue
    /// time. Lower bound on the eventual fanout-group signature; used by
    /// the estimation layer to look up the matching group's EMA.
    pub(crate) base_signature: Vec<MethodId>,
    pub(crate) service_path_prefix: PathPrefix,
    pub(crate) base_service_signature: Vec<MethodId>,
    #[allow(dead_code)]
    pub(crate) child_id: MethodId,
    start_time: Instant,
}

impl ChildRPCTracker {
    fn new(
        key: ParentToChildKey,
        child_id: MethodId,
        child_index: usize,
        path_prefix: PathPrefix,
        base_signature: Vec<MethodId>,
        service_path_prefix: PathPrefix,
        base_service_signature: Vec<MethodId>,
        start_time: Instant,
    ) -> Self {
        Self {
            key,
            child_index,
            path_prefix,
            base_signature,
            service_path_prefix,
            base_service_signature,
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
    // A single user-facing request that signals at multiple hops should still
    // count as one event; otherwise a 5-hop request that signals at every hop
    // looks like 5 separate failures. We track presence (AtomicBool), not a
    // running tally, so the ingress sees deadline_signal_count in {0, 1}.
    child_deadline_signal: AtomicBool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyEwma;
    use tonic_core::CowGrpcMethod;

    fn mid(service: &str, method: &str) -> MethodId {
        MethodRegistry::global()
            .get_or_register(CowGrpcMethod::new(service.to_string(), method.to_string()))
    }

    fn sid(service: &str) -> MethodId {
        MethodRegistry::global().get_or_register_service(service.to_string())
    }

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

    #[test]
    fn fanout_deadline_blend_moves_lower_estimates_toward_legacy() {
        assert_eq!(blend_toward_legacy(1_000, 5_000, 0.0), 1_000);
        assert_eq!(blend_toward_legacy(1_000, 5_000, 0.5), 3_000);
        assert_eq!(blend_toward_legacy(1_000, 5_000, 1.0), 5_000);
        assert_eq!(blend_toward_legacy(6_000, 5_000, 0.5), 5_000);
    }

    #[test]
    fn fanout_state_advances_observable_prefix_before_open_group() {
        let a = mid("StateFanout", "a");
        let b = mid("StateFanout", "b");
        let c = mid("StateFanout", "c");
        let t0 = Instant::now();
        let mut state = FanoutInvocationState::default();
        state.children.push(ChildRecord {
            child_id: a,
            child_service_id: sid("StateFanout"),
            start: t0,
            end: Some(t0 + Duration::from_millis(10)),
            terminal: true,
        });
        state.children.push(ChildRecord {
            child_id: b,
            child_service_id: sid("StateFanout"),
            start: t0 + Duration::from_millis(1),
            end: Some(t0 + Duration::from_millis(20)),
            terminal: true,
        });
        state.children.push(ChildRecord {
            child_id: c,
            child_service_id: sid("StateFanout"),
            start: t0 + Duration::from_millis(30),
            end: None,
            terminal: false,
        });

        state.refresh_path_prefix_before_open_group(t0 + Duration::from_millis(30));

        let mut sig = vec![a, b];
        sig.sort();
        assert_eq!(state.path_prefix, PathPrefix::root().append_signature(&sig));
        assert_eq!(state.incorporated_child_count, 2);
    }

    #[test]
    fn estimation_tracker_issues_open_group_with_completed_prefix() {
        let root = mid("StateTracker", "root");
        let parent = mid("StateTracker", "parent");
        let child_a = CowGrpcMethod::new("StateTracker", "child_a");
        let child_b = CowGrpcMethod::new("StateTracker", "child_b");
        let child_c = CowGrpcMethod::new("StateTracker", "child_c");
        let child_a_id = MethodRegistry::global().get_or_register(child_a.clone());
        let child_b_id = MethodRegistry::global().get_or_register(child_b.clone());
        let child_service_id = MethodRegistry::global().get_or_register_service("StateTracker");
        let est = LatencyEstimators::<LatencyEwma>::new();
        let tracker = EstimationTracker::new(parent, Some(root), est);

        let first = tracker.begin_child(&child_a);
        let response: Result<Response<()>, Status> = Ok(Response::new(()));
        tracker.record_child_complete(&first, &response);

        let second = tracker.begin_child(&child_b);
        assert_eq!(second.path_prefix, PathPrefix::root());

        let third = tracker.begin_child(&child_c);
        assert_eq!(
            third.path_prefix,
            PathPrefix::root().append_signature(&[child_a_id])
        );
        assert_eq!(third.base_signature, {
            let mut sig = vec![child_b_id, third.child_id];
            sig.sort();
            sig
        });
        assert_eq!(third.base_service_signature, {
            let mut sig = vec![child_service_id, child_service_id];
            sig.sort();
            sig
        });
    }

    #[test]
    fn fanout_lookup_clamps_to_legacy_when_fanout_is_higher() {
        let root = mid("StateLookupClamp", "root");
        let parent = mid("StateLookupClamp", "parent");
        let child = mid("StateLookupClamp", "child");
        let sibling = mid("StateLookupClamp", "sibling");
        let service = sid("StateLookupClamp");
        let est = LatencyEstimators::<LatencyEwma>::new();
        let mut signature = vec![child, sibling];
        signature.sort();
        let mut service_signature = vec![service, service];
        service_signature.sort();
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(parent)
            .child_rpc_method(child);

        est.track_after_child_wallclock(key, 1_000);
        for _ in 0..3 {
            est.track_fanout_group(
                root,
                parent,
                PathPrefix::root(),
                signature.clone(),
                PathPrefix::root(),
                service_signature.clone(),
                5_000,
            );
        }

        let estimate = est.est_after_child_wallclock_for_group(
            root,
            parent,
            PathPrefix::root(),
            &signature,
            PathPrefix::root(),
            &service_signature,
            child,
            u64::MAX,
        );

        assert_eq!(estimate.full, 1_000);
        assert_eq!(estimate.mean, 1_000);
        assert_eq!(estimate.floor, 1_000);
    }

    #[test]
    fn fanout_lookup_uses_fanout_when_lower_than_legacy() {
        let root = mid("StateLookupMin", "root");
        let parent = mid("StateLookupMin", "parent");
        let child = mid("StateLookupMin", "child");
        let sibling = mid("StateLookupMin", "sibling");
        let service = sid("StateLookupMin");
        let est = LatencyEstimators::<LatencyEwma>::new();
        let mut signature = vec![child, sibling];
        signature.sort();
        let mut service_signature = vec![service, service];
        service_signature.sort();
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(parent)
            .child_rpc_method(child);

        est.track_after_child_wallclock(key, 5_000);
        for _ in 0..3 {
            est.track_fanout_group(
                root,
                parent,
                PathPrefix::root(),
                signature.clone(),
                PathPrefix::root(),
                service_signature.clone(),
                1_000,
            );
        }

        let estimate = est.est_after_child_wallclock_for_group(
            root,
            parent,
            PathPrefix::root(),
            &signature,
            PathPrefix::root(),
            &service_signature,
            child,
            u64::MAX,
        );

        assert_eq!(estimate.full, 5_000);
        assert_eq!(estimate.mean, 1_000);
        assert_eq!(estimate.floor, 1_000);
    }

    #[test]
    fn fanout_lookup_falls_back_to_service_shape_when_exact_is_sparse() {
        let root = mid("StateLookupCoarseRoot", "root");
        let parent = mid("StateLookupCoarseParent", "parent");
        let a1 = mid("StateLookupCoarseA", "a1");
        let a2 = mid("StateLookupCoarseA", "a2");
        let a3 = mid("StateLookupCoarseA", "a3");
        let b1 = mid("StateLookupCoarseB", "b1");
        let b2 = mid("StateLookupCoarseB", "b2");
        let b3 = mid("StateLookupCoarseB", "b3");
        let service_a = sid("StateLookupCoarseA");
        let service_b = sid("StateLookupCoarseB");
        let est = LatencyEstimators::<LatencyEwma>::new();
        let mut service_signature = vec![service_a, service_b];
        service_signature.sort();
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(parent)
            .child_rpc_method(a1);

        est.track_after_child_wallclock(key, 5_000);
        for (left, right) in [(a1, b1), (a2, b2), (a3, b3)] {
            let mut exact_signature = vec![left, right];
            exact_signature.sort();
            est.track_fanout_group(
                root,
                parent,
                PathPrefix::root(),
                exact_signature,
                PathPrefix::root(),
                service_signature.clone(),
                1_000,
            );
        }

        let mut cold_exact = vec![a1, b1];
        cold_exact.sort();
        let estimate = est.est_after_child_wallclock_for_group(
            root,
            parent,
            PathPrefix::root(),
            &cold_exact,
            PathPrefix::root(),
            &service_signature,
            a1,
            u64::MAX,
        );

        assert_eq!(estimate.full, 5_000);
        assert_eq!(estimate.mean, 1_000);
        assert_eq!(estimate.floor, 1_000);
    }

    #[test]
    fn fanout_lookup_ignores_single_child_groups() {
        let root = mid("StateLookupSingle", "root");
        let parent = mid("StateLookupSingle", "parent");
        let child = mid("StateLookupSingle", "child");
        let service = sid("StateLookupSingle");
        let est = LatencyEstimators::<LatencyEwma>::new();
        let key = ParentToChildKey::root_rpc_method(root)
            .parent_rpc_method(parent)
            .child_rpc_method(child);

        est.track_after_child_wallclock(key, 5_000);
        est.track_fanout_group(
            root,
            parent,
            PathPrefix::root(),
            vec![child],
            PathPrefix::root(),
            vec![service],
            1_000,
        );

        let estimate = est.est_after_child_wallclock_for_group(
            root,
            parent,
            PathPrefix::root(),
            &[child],
            PathPrefix::root(),
            &[service],
            child,
            u64::MAX,
        );

        assert_eq!(estimate.full, 5_000);
        assert_eq!(estimate.mean, 5_000);
        assert_eq!(estimate.floor, 5_000);
    }

    #[test]
    fn deadline_signal_propagates_to_response_meta() {
        let tracker = RequestMetadataTracker::new();
        tracker.mark_deadline_signal();

        let mut ctx = Context::default();
        tracker.inject_response_meta(&mut ctx);

        let meta = ctx.response_meta().expect("response_meta set");
        assert_eq!(meta.deadline_signal_count, 1);
        assert_eq!(meta.early_return_count, 0);
    }

    #[test]
    fn deadline_signal_default_is_zero_when_not_marked() {
        let tracker = RequestMetadataTracker::new();
        let mut ctx = Context::default();
        tracker.inject_response_meta(&mut ctx);

        let meta = ctx.response_meta().expect("response_meta set");
        assert_eq!(meta.deadline_signal_count, 0);
    }

    #[test]
    fn mark_deadline_signal_is_idempotent() {
        let tracker = RequestMetadataTracker::new();
        tracker.mark_deadline_signal();
        tracker.mark_deadline_signal();
        tracker.mark_deadline_signal();

        let mut ctx = Context::default();
        tracker.inject_response_meta(&mut ctx);

        assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
    }

    #[test]
    fn deadline_signal_saturates_across_children() {
        // A request whose local deadline trips AND whose children also signal
        // should still count as one event (not 1 + N children).
        let tracker = RequestMetadataTracker::new();
        tracker.mark_deadline_signal();
        // Simulate absorbing several children that each reported a signal.
        for _ in 0..5 {
            tracker.child_deadline_signal.store(true, Ordering::Relaxed);
        }

        let mut ctx = Context::default();
        tracker.inject_response_meta(&mut ctx);

        assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
    }

    #[test]
    fn deadline_signal_propagates_from_child_only() {
        // Even if the local hop did not signal, a single descendant signal
        // should still surface as 1 at this hop's response_meta.
        let tracker = RequestMetadataTracker::new();
        tracker.child_deadline_signal.store(true, Ordering::Relaxed);

        let mut ctx = Context::default();
        tracker.inject_response_meta(&mut ctx);

        assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
    }
}
