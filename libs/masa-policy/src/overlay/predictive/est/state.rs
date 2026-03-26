// Latency estimation state for Masa scheduling policies.
//
// Provides `EstServerState`, `EstRequestState`, and `EstChildState` that can be
// embedded into any policy's Server/Parent/ChildContext to track latency
// distributions and make abort decisions. Used by:
// - pred_sched.rs: deadline tightening and dynamic reprioritization
// - ac/predictive_ac.rs: reads estimation maps for admission control decisions
// - standard.rs: orchestrates estimation and admission control

use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

use masa_core::{time_now, Context, LatencyEstimator, ResponseMeta};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::estimator::ParentToChildId;
use super::latency_map::{spawn_method_stats_printer, spawn_stats_printer, LatencyMap};
use crate::context_ext::{MasaResponseExt, MasaStatusExt};
use crate::MethodRegistry;

/// Server-level estimation state (shared across requests on a service).
#[derive(Debug)]
pub(crate) struct EstServerState<E: LatencyEstimator + Default + 'static> {
    /// Tracks remaining duration after each child RPC completes (keyed by parent->child pair).
    pub est_after_child_latency: Arc<LatencyMap<E>>,
    /// Tracks actual child RPC call latencies (keyed by parent->child pair).
    pub est_child_latency: Arc<LatencyMap<E>>,
    /// Tracks compute time (poll duration) per method.
    pub est_compute_latency: Arc<LatencyMap<E>>,
    /// Counter for periodic logging.
    pub print_counter: AtomicUsize,
}

impl<E: LatencyEstimator + Default + 'static> EstServerState<E> {
    pub(crate) fn new() -> Self {
        let est_after_child_latency = Arc::new(LatencyMap::new());
        let est_child_latency = Arc::new(LatencyMap::new());
        let est_compute_latency = Arc::new(LatencyMap::new());

        spawn_stats_printer(est_after_child_latency.clone(), "Est Remaining Values");
        spawn_stats_printer(est_child_latency.clone(), "Est Child Call Latencies");
        spawn_method_stats_printer(est_compute_latency.clone(), "Est Compute Latencies");

        Self {
            est_after_child_latency,
            est_child_latency,
            est_compute_latency,
            print_counter: AtomicUsize::new(0),
        }
    }
}

/// Per-request estimation state.
#[derive(Debug)]
pub(crate) struct EstRequestState<E: LatencyEstimator + Default + 'static> {
    pub resolved_method_id: u64,
    pub server: Arc<EstServerState<E>>,
    pub child_end_times: Mutex<Vec<(ParentToChildId, Instant)>>,
    pub poll_compute_us: AtomicU64,
    pub poll_start: Mutex<Option<Instant>>,
    pub max_child_downstream_util: Mutex<f32>,
}

impl<E: LatencyEstimator + Default + 'static> EstRequestState<E> {
    pub(crate) fn new(resolved_method_id: u64, server: Arc<EstServerState<E>>) -> Self {
        Self {
            resolved_method_id,
            server,
            child_end_times: Mutex::new(Vec::new()),
            poll_compute_us: AtomicU64::new(0),
            poll_start: Mutex::new(None),
            max_child_downstream_util: Mutex::new(0.0),
        }
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
        for (parent_to_child_id, child_end) in child_end_times {
            self.server.est_after_child_latency.track(
                parent_to_child_id.to_key(),
                parent_end.duration_since(child_end).as_micros() as u64,
            );
        }

        // Track compute time (accumulated poll duration) per method
        self.server.est_compute_latency.track(
            self.resolved_method_id,
            self.poll_compute_us.load(Ordering::Relaxed),
        );
    }

    /// Inject ResponseMeta (compute time, utilization) into the response.
    pub(crate) fn inject_response_meta<Ret>(
        &self,
        ctx: &Context,
        result: &mut Result<Response<Ret>, Status>,
    ) {
        let compute_time_us = self.poll_compute_us.load(Ordering::Relaxed);
        let utilization = tokio::task::current_utilization() as f32;
        let max_child_util = *self.max_child_downstream_util.lock().unwrap();
        let max_downstream_util = utilization.max(max_child_util);

        let mut ctx = ctx.clone();
        ctx.set_response_meta(ResponseMeta {
            compute_time_us,
            utilization,
            max_downstream_util,
        });

        match result {
            Ok(resp) => {
                resp.set_masa_context(&ctx);
            }
            Err(status) => {
                status.set_masa_context(&ctx);
            }
        }
    }

    /// Process a child RPC response: extract ResponseMeta, track latencies,
    /// handle early-return negative feedback, and record child end time.
    ///
    /// Returns the `max_downstream_util` from the child response metadata,
    /// if available, so the caller can feed it to admission control.
    pub(crate) fn after_child_rpc<T>(
        &self,
        response: &Result<Response<T>, Status>,
        child_ctx: &EstChildState<E>,
    ) -> Option<f32> {
        let mut downstream_util = None;

        // Extract ResponseMeta from child response
        if let Ok(resp) = response {
            if let Some(child_ctx_resp) = resp.get_masa_context() {
                if let Some(meta) = child_ctx_resp.response_meta() {
                    // Track child's compute time
                    if let Some(parent_to_child_id) = &child_ctx.parent_to_child_id {
                        self.server
                            .est_compute_latency
                            .track(parent_to_child_id.to_key(), meta.compute_time_us);
                    }
                    // Track max downstream utilization
                    let mut max_util = self.max_child_downstream_util.lock().unwrap();
                    if meta.max_downstream_util > *max_util {
                        *max_util = meta.max_downstream_util;
                    }
                    downstream_util = Some(meta.max_downstream_util);
                }
            }
        }

        // When the child early-returns, track 0 into est_after_child_latency to create
        // negative feedback. Without this, ERs prevent tracking updates, freezing the
        // estimate at a high value.
        if let Err(status) = response {
            if status.code() == Code::DeadlineExceeded {
                if let Some(parent_to_child_id) = &child_ctx.parent_to_child_id {
                    self.server
                        .est_after_child_latency
                        .track(parent_to_child_id.to_key(), 0);
                }
            }
        }

        // Record child end time for successful responses
        if response.is_ok() {
            if let Some(parent_to_child_id) = &child_ctx.parent_to_child_id {
                self.child_end_times
                    .lock()
                    .unwrap()
                    .push((parent_to_child_id.clone(), Instant::now()));
            }
        }

        downstream_util
    }

    /// Periodic logging of latency estimates.
    pub(crate) fn log_estimates(
        &self,
        parent_to_child_id: &ParentToChildId,
        key: u64,
        est_remaining: u64,
        est_remaining_mean: u64,
        est_remaining_floor: u64,
    ) {
        if self.server.print_counter.fetch_add(1, Ordering::Relaxed) % 5000 == 0 {
            let est_child = self.server.est_child_latency.get_estimate(key).unwrap_or(0);
            log::info!(
                "LAT_EST: p=>c: {}, est_child: {}, est_rem: {}, est_rem_mean: {}, est_rem_floor: {}",
                parent_to_child_id,
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
        let parent_to_child_id = ParentToChildId {
            parent_id: self.resolved_method_id,
            child_id: resolved_child_id,
        };
        let key = parent_to_child_id.to_key();

        child_est.setup(
            parent_to_child_id.clone(),
            child_method_name.clone(),
            self.server.clone(),
        );

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        let est_remaining = self
            .server
            .est_after_child_latency
            .get_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        let est_remaining_mean = self
            .server
            .est_after_child_latency
            .get_mean_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        let est_remaining_floor = self
            .server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        self.log_estimates(
            &parent_to_child_id,
            key,
            est_remaining,
            est_remaining_mean,
            est_remaining_floor,
        );

        ChildRpcPrepareResult { est_remaining, key }
    }

    /// Floor-based admission check using latency estimates.
    ///
    /// Returns true if the request should be shed (rejected).
    #[allow(dead_code)]
    pub(crate) fn admission_check(&self, ctx: &Context, key: u64) -> bool {
        use masa_core::time_now;

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());
        let est_remaining_floor = self
            .server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        time_now() > ctx.e2e_deadline().saturating_sub(est_remaining_floor)
    }
}

/// Result of `prepare_before_child_rpc` containing estimates for deadline computation.
#[allow(dead_code)]
pub(crate) struct ChildRpcPrepareResult {
    pub est_remaining: u64,
    /// Parent->child key for admission control lookups.
    pub key: u64,
}

/// Per-child-RPC estimation state.
#[derive(Debug, Clone)]
pub(crate) struct EstChildState<E: LatencyEstimator + Default + 'static> {
    pub start_time: Option<Instant>,
    pub parent_to_child_id: Option<ParentToChildId>,
    pub child_method: Option<CowGrpcMethod>,
    pub server: Option<Arc<EstServerState<E>>>,
}

impl<E: LatencyEstimator + Default + 'static> EstChildState<E> {
    pub(crate) fn new() -> Self {
        Self {
            start_time: None,
            parent_to_child_id: None,
            child_method: None,
            server: None,
        }
    }

    pub(crate) fn setup(
        &mut self,
        parent_to_child_id: ParentToChildId,
        child_method: CowGrpcMethod,
        server: Arc<EstServerState<E>>,
    ) {
        self.start_time = Some(Instant::now());
        self.parent_to_child_id = Some(parent_to_child_id);
        self.child_method = Some(child_method);
        self.server = Some(server);
    }

    pub(crate) fn finalize<T>(&self, response: &Result<Response<T>, Status>) {
        if is_early_return_response(response) {
            return;
        }

        if let (Some(start_time), Some(parent_to_child_id), Some(server)) =
            (self.start_time, &self.parent_to_child_id, &self.server)
        {
            let client_runtime = Instant::now().duration_since(start_time).as_micros() as u64;
            server
                .est_child_latency
                .track(parent_to_child_id.to_key(), client_runtime);
        }
    }
}

pub(crate) fn is_early_return_response<T>(response: &Result<Response<T>, Status>) -> bool {
    match response {
        Ok(_) => false,
        Err(status) => status.code() == Code::DeadlineExceeded,
    }
}
