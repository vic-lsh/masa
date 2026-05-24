use std::fmt;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;

use masa_core::LatencyEstimator;

use super::super::fanout::{FanoutPatternTable, PathPrefix, PatternScope};
use super::super::latency_map::{LatencyMap, MethodKey, ParentToChildKey, RootToLocalKey};
use crate::policy_params::PolicyParams;
use crate::registry::MethodId;

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

pub(super) fn blend_toward_legacy(value: u64, legacy: u64, fraction: f64) -> u64 {
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

pub(super) fn fanout_enabled_for_parent(root: MethodId, parent: MethodId) -> bool {
    if !fanout_aware_enabled() {
        return false;
    }
    let params = &PolicyParams::global().pred;
    !params.fanout_root_only || root == parent
}

/// Runtime toggle for verbose estimator table dumps.
///
/// The fanout table can contain thousands of method-level signatures on tracebench
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
    ) -> Option<super::super::latency_map::EstimatesPack> {
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
        let root_name = super::super::latency_map::format_method_name(root);
        let parent_name = super::super::latency_map::format_method_name(parent);
        let scope_name = match scope {
            PatternScope::Aggregate => "agg".to_string(),
            PatternScope::Path(prefix) => format!("p={}", prefix.label()),
        };
        let sig_str = signature
            .iter()
            .map(|id| super::super::latency_map::format_method_name(*id))
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
        let name = super::super::latency_map::format_method_name(key.0);
        if distribution.can_estimate() {
            parts.push(format!("{}: {} us", name, distribution.estimate()));
        } else {
            parts.push(format!("{}: (no estimate)", name));
        }
    });
    log::info!("{}: {}", label, parts.join(", "));
}
