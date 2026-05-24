use std::sync::Mutex;
use std::time::Instant;

use masa_core::LatencyEstimator;
use tonic_core::{CowGrpcMethod, Response, Status};

use super::super::fanout::{recover_path_groups, ChildRecord, PathPrefix};
use super::super::latency_map::{ParentToChildKey, RootToLocalKey};
use super::latency_estimators::{fanout_enabled_for_parent, LatencyEstimators};
use super::metadata::{is_early_return_response, is_signaled_response};
use crate::registry::MethodId;
use crate::MethodRegistry;

// ══════════════════════════════════════════════════════════════════════════
// Per-request estimation lifecycle
// ══════════════════════════════════════════════════════════════════════════

/// Per-parent invocation state for deriving non-invasive fanout path prefixes.
#[derive(Debug, Default)]
pub(super) struct FanoutInvocationState {
    /// All child RPCs issued under this handler invocation, in issue order.
    pub(super) children: Vec<ChildRecord>,
    /// Prefix formed by completed fanout groups before the current open group.
    pub(super) path_prefix: PathPrefix,
    /// Service-shape version of `path_prefix`.
    service_path_prefix: PathPrefix,
    /// Number of leading child records already folded into `path_prefix`.
    pub(super) incorporated_child_count: usize,
}

impl FanoutInvocationState {
    pub(super) fn refresh_path_prefix_before_open_group(&mut self, now: Instant) {
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
