// Latency estimation state for Masa scheduling policies.
//
// Provides `EstServerState`, `EstRequestState`, and `EstChildState` that can be
// embedded into any policy's Server/Parent/ChildContext to track latency
// distributions and make abort decisions. Used by `PredAdmissionLayer`
// for deadline tightening, dynamic reprioritization, and admission control.

use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

use masa_core::{time_now, Context, LatencyEstimator, ResponseMeta};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::latency_map::{
    spawn_method_stats_printer, spawn_pair_stats_printer, LatencyMap, MethodKey, ParentToChildKey,
    RootToLocalKey,
};
use crate::context_ext::MasaResponseExt;
use crate::registry::MethodId;
use crate::MethodRegistry;

/// Server-level estimation state (shared across requests on a service).
#[derive(Debug)]
pub(crate) struct EstServerState<E: LatencyEstimator + Default + 'static> {
    /// Tracks remaining duration after each child RPC completes (keyed by parent->child pair).
    pub est_after_child_latency: Arc<LatencyMap<ParentToChildKey, E>>,
    /// Tracks actual child RPC call latencies (keyed by parent->child pair).
    pub est_child_latency: Arc<LatencyMap<ParentToChildKey, E>>,
    /// EMA of accumulated compute cost per root API type (for Layer 2 capacity metering).
    pub est_accumulated_cost: Arc<LatencyMap<MethodKey, E>>,
    /// Total wall-clock latency per method keyed by (root API type, local method) for early feasibility.
    pub est_method_latency: Arc<LatencyMap<RootToLocalKey, E>>,
    /// Counter for periodic logging.
    pub print_counter: AtomicUsize,
}

impl<E: LatencyEstimator + Default + 'static> EstServerState<E> {
    pub(crate) fn new() -> Self {
        let est_after_child_latency = Arc::new(LatencyMap::new());
        let est_child_latency = Arc::new(LatencyMap::new());
        let est_accumulated_cost = Arc::new(LatencyMap::new());
        let est_method_latency = Arc::new(LatencyMap::new());

        spawn_pair_stats_printer(est_after_child_latency.clone(), "Est Remaining Values");
        spawn_pair_stats_printer(est_child_latency.clone(), "Est Child Call Latencies");
        spawn_method_stats_printer(est_accumulated_cost.clone(), "Est Accumulated Cost");
        spawn_pair_stats_printer(est_method_latency.clone(), "Est Method Latency");

        Self {
            est_after_child_latency,
            est_child_latency,
            est_accumulated_cost,
            est_method_latency,
            print_counter: AtomicUsize::new(0),
        }
    }
}

/// Information extracted from a child RPC response.
#[allow(dead_code)]
pub(crate) struct ChildResponseInfo {
    pub downstream_util: Option<f32>,
    pub accumulated_compute_us: Option<u64>,
}

/// Per-request estimation state.
#[derive(Debug)]
pub(crate) struct EstRequestState<E: LatencyEstimator + Default + 'static> {
    pub resolved_method_id: MethodId,
    pub root_method_id: Option<MethodId>,
    pub server: Arc<EstServerState<E>>,
    pub child_end_times: Mutex<Vec<(ParentToChildKey, Instant)>>,
    pub poll_compute_us: AtomicU64,
    pub poll_start: Mutex<Option<Instant>>,
    pub max_child_downstream_util: Mutex<f32>,
    pub accumulated_child_compute_us: AtomicU64,
    pub request_start: Instant,
}

impl<E: LatencyEstimator + Default + 'static> EstRequestState<E> {
    pub(crate) fn new(
        resolved_method_id: MethodId,
        root_method_id: Option<MethodId>,
        server: Arc<EstServerState<E>>,
    ) -> Self {
        Self {
            resolved_method_id,
            root_method_id,
            server,
            child_end_times: Mutex::new(Vec::new()),
            poll_compute_us: AtomicU64::new(0),
            poll_start: Mutex::new(None),
            max_child_downstream_util: Mutex::new(0.0),
            accumulated_child_compute_us: AtomicU64::new(0),
            request_start: Instant::now(),
        }
    }

    /// Estimated wall-clock latency for this (root API type, local method) pair.
    pub(crate) fn est_method_latency(&self) -> Option<u64> {
        let key = self.method_latency_key()?;
        self.server.est_method_latency.get_estimate(key)
    }

    /// Compound key for tracking into `est_method_latency`.
    fn method_latency_key(&self) -> Option<RootToLocalKey> {
        let root = self.root_method_id?;
        Some(RootToLocalKey::root_rpc_method(root).local_rpc_method(self.resolved_method_id))
    }

    /// Start tracking compute time for the current poll.
    pub(crate) fn start_compute_tracking(&self) {
        *self.poll_start.lock().unwrap() = Some(Instant::now());
    }

    /// Stop tracking compute time and accumulate elapsed time.
    pub(crate) fn stop_compute_tracking(&self) {
        if let Some(start) = self.poll_start.lock().unwrap().take() {
            let elapsed_us = start.elapsed().as_micros() as u64;
            self.poll_compute_us
                .fetch_add(elapsed_us, Ordering::Relaxed);
        }
    }

    /// Track latency observations for completed request.
    pub(crate) fn track_latencies(&self) {
        let parent_end = Instant::now();

        // Track remaining time after child RPC completes
        let child_end_times = std::mem::take(&mut *self.child_end_times.lock().unwrap());
        for (parent_to_child_key, child_end) in child_end_times {
            self.server.est_after_child_latency.track(
                parent_to_child_key,
                parent_end.duration_since(child_end).as_micros() as u64,
            );
        }

        // Track total wall-clock latency per (root API type, local method) for early feasibility.
        if let Some(key) = self.method_latency_key() {
            let total_wall_clock = self.request_start.elapsed().as_micros() as u64;
            self.server.est_method_latency.track(key, total_wall_clock);
        }
    }

    /// Set ResponseMeta (compute time, utilization) on the shared context.
    ///
    /// The caller is responsible for serializing `ctx` into the response;
    /// this method only mutates the in-memory context.
    pub(crate) fn inject_response_meta(&self, ctx: &mut Context) {
        let compute_time_us = self.poll_compute_us.load(Ordering::Relaxed);
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

    /// Process a child RPC response: extract ResponseMeta, track latencies,
    /// handle early-return negative feedback, and record child end time.
    ///
    /// Returns child response info including downstream utilization and
    /// accumulated compute cost, so the caller can feed them to admission control.
    pub(crate) fn after_child_rpc<T>(
        &self,
        response: &Result<Response<T>, Status>,
        child_ctx: &EstChildState<E>,
    ) -> ChildResponseInfo {
        let mut downstream_util = None;
        let mut accumulated_compute_us = None;

        // Extract ResponseMeta from child response
        if let Ok(resp) = response {
            if let Some(child_ctx_resp) = resp.get_masa_context() {
                if let Some(meta) = child_ctx_resp.response_meta() {
                    // Track max downstream utilization
                    let mut max_util = self.max_child_downstream_util.lock().unwrap();
                    if meta.max_downstream_util > *max_util {
                        *max_util = meta.max_downstream_util;
                    }
                    downstream_util = Some(meta.max_downstream_util);
                    // Accumulate child's total tree compute cost
                    self.accumulated_child_compute_us
                        .fetch_add(meta.accumulated_compute_us, Ordering::Relaxed);
                    accumulated_compute_us = Some(meta.accumulated_compute_us);
                }
            }
        }

        // When the child early-returns, track 0 into est_after_child_latency to create
        // negative feedback. Without this, ERs prevent tracking updates, freezing the
        // estimate at a high value.
        if let Err(status) = response {
            if status.code() == Code::DeadlineExceeded {
                if let Some(key) = &child_ctx.parent_to_child_key {
                    self.server.est_after_child_latency.track(*key, 0);
                }
            }
        }

        // Record child end time for successful responses
        if response.is_ok() {
            if let Some(key) = &child_ctx.parent_to_child_key {
                self.child_end_times
                    .lock()
                    .unwrap()
                    .push((*key, Instant::now()));
            }
        }

        ChildResponseInfo {
            downstream_util,
            accumulated_compute_us,
        }
    }

    /// Periodic logging of latency estimates.
    pub(crate) fn log_estimates(
        &self,
        parent_to_child_key: &ParentToChildKey,
        est_remaining: u64,
        est_remaining_mean: u64,
        est_remaining_floor: u64,
    ) {
        if self.server.print_counter.fetch_add(1, Ordering::Relaxed) % 5000 == 0 {
            let est_child = self
                .server
                .est_child_latency
                .get_estimate(*parent_to_child_key)
                .unwrap_or(0);
            log::info!(
                "LAT_EST: p=>c: {}, est_child: {}, est_rem: {}, est_rem_mean: {}, est_rem_floor: {}",
                parent_to_child_key,
                est_child,
                est_remaining,
                est_remaining_mean,
                est_remaining_floor,
            );
        }
    }

    /// Setup child context and log estimates.
    /// Returns estimates for deadline computation and admission control lookups.
    pub(crate) fn prepare_before_child_rpc(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        child_est: &mut EstChildState<E>,
    ) -> ChildRpcPrepareResult {
        let resolved_child_id = MethodRegistry::global()
            .get_or_register_method(child_method_name.service(), child_method_name.method());
        let parent_to_child_key = ParentToChildKey::parent_rpc_method(self.resolved_method_id)
            .child_rpc_method(resolved_child_id);

        child_est.setup(
            parent_to_child_key,
            child_method_name.clone(),
            self.server.clone(),
        );

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        let est_remaining = self
            .server
            .est_after_child_latency
            .get_estimate(parent_to_child_key)
            .unwrap_or(0)
            .min(time_left);

        let est_remaining_mean = self
            .server
            .est_after_child_latency
            .get_mean_estimate(parent_to_child_key)
            .unwrap_or(0)
            .min(time_left);

        let est_remaining_floor = self
            .server
            .est_after_child_latency
            .get_mean_floor_estimate(parent_to_child_key)
            .unwrap_or(0)
            .min(time_left);

        self.log_estimates(
            &parent_to_child_key,
            est_remaining,
            est_remaining_mean,
            est_remaining_floor,
        );

        ChildRpcPrepareResult {
            est_remaining,
            parent_to_child_key,
        }
    }
}

/// Result of `prepare_before_child_rpc` containing estimates for deadline computation.
#[allow(dead_code)]
pub(crate) struct ChildRpcPrepareResult {
    pub est_remaining: u64,
    /// Parent->child key for admission control lookups.
    pub parent_to_child_key: ParentToChildKey,
}

/// Per-child-RPC estimation state.
#[derive(Debug, Clone)]
pub(crate) struct EstChildState<E: LatencyEstimator + Default + 'static> {
    pub start_time: Option<Instant>,
    pub parent_to_child_key: Option<ParentToChildKey>,
    pub child_method: Option<CowGrpcMethod>,
    pub server: Option<Arc<EstServerState<E>>>,
}

impl<E: LatencyEstimator + Default + 'static> EstChildState<E> {
    pub(crate) fn new() -> Self {
        Self {
            start_time: None,
            parent_to_child_key: None,
            child_method: None,
            server: None,
        }
    }

    pub(crate) fn setup(
        &mut self,
        parent_to_child_key: ParentToChildKey,
        child_method: CowGrpcMethod,
        server: Arc<EstServerState<E>>,
    ) {
        self.start_time = Some(Instant::now());
        self.parent_to_child_key = Some(parent_to_child_key);
        self.child_method = Some(child_method);
        self.server = Some(server);
    }

    pub(crate) fn finalize<T>(&self, response: &Result<Response<T>, Status>) {
        if is_early_return_response(response) {
            return;
        }

        if let (Some(start_time), Some(key), Some(server)) =
            (self.start_time, &self.parent_to_child_key, &self.server)
        {
            let client_runtime = Instant::now().duration_since(start_time).as_micros() as u64;
            server.est_child_latency.track(*key, client_runtime);
        }
    }
}

pub(crate) fn is_early_return_response<T>(response: &Result<Response<T>, Status>) -> bool {
    match response {
        Ok(_) => false,
        Err(status) => status.code() == Code::DeadlineExceeded,
    }
}
