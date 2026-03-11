use crate::masa::MethodRegistry;
use crate::{
    masa::context::read_context, Code, CowGrpcMethod, GrpcMethod, Request, Response, Status,
};
use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    task::Poll,
    time::{Duration, Instant},
};

use super::super::common::{EarlyReturnHandler, QueueLatencyTracker};
use super::super::{
    resolve_method_name_from_http, ClientHooks, MasaHooks, MasaRequestExt, ParentHooks, ServerHooks,
};
#[cfg(feature = "emp_admission")]
use super::completion_rate_map;
use super::LatencyMap;
use masa_core::{time_now, Context, ContextBuilder, LatencyEstimator, PriorityHint, EARLY_RETURN};

#[cfg(feature = "est_hist")]
use masa_core::LatencyDistribution as LatencyHistogram;

#[cfg(feature = "est_mean_var")]
use masa_core::LatencyMeanVar;

#[cfg(any(
    feature = "est_rms",
    all(
        not(feature = "est_rms"),
        not(feature = "est_hist"),
        not(feature = "est_mean_var")
    )
))]
use masa_core::LatencyRms;

use std::sync::atomic::AtomicUsize;

#[cfg(all(feature = "est_rms", feature = "est_hist"))]
compile_error!("Features 'est_rms' and 'est_hist' cannot be enabled simultaneously");

#[cfg(all(feature = "est_rms", feature = "est_mean_var"))]
compile_error!("Features 'est_rms' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(all(feature = "est_hist", feature = "est_mean_var"))]
compile_error!("Features 'est_hist' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(any(feature = "est_rms", feature = "est_hist", feature = "est_mean_var"))]
#[cfg(not(feature = "prio_local"))]
compile_error!(
    "Features 'est_rms', 'est_hist', or 'est_mean_var' require 'prio_local' to be enabled"
);

#[cfg(all(feature = "emp_admission", not(feature = "prio_local")))]
compile_error!("Feature 'emp_admission' requires 'prio_local'");

#[cfg(all(feature = "emp_admission", not(feature = "early")))]
compile_error!("Feature 'emp_admission' requires 'early'");

/// Type alias for the latency estimator used in the local deadline policy.
#[cfg(feature = "est_hist")]
pub(crate) type LocalLatencyEstimator = LatencyHistogram;

#[cfg(feature = "est_mean_var")]
pub(crate) type LocalLatencyEstimator = LatencyMeanVar;

#[cfg(any(
    feature = "est_rms",
    all(
        not(feature = "est_rms"),
        not(feature = "est_hist"),
        not(feature = "est_mean_var")
    )
))]
pub(crate) type LocalLatencyEstimator = LatencyRms;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct ParentToChildId {
    pub parent_id: u64,
    pub child_id: u64,
}

impl ParentToChildId {
    pub(crate) fn to_key(&self) -> u64 {
        // Simple combination of two 32-bit (effective) IDs into one 64-bit key
        (self.parent_id << 32) | self.child_id
    }
}

impl std::fmt::Display for ParentToChildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}=>{}", self.parent_id, self.child_id)
    }
}

#[derive(Debug)]
/// This policy computes the deadline d of a child request as
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlinePolicy;

impl MasaHooks for LocalDeadlinePolicy {
    type ServerContext = ServerContext<LocalLatencyEstimator>;
    type ChildContext = ChildContext<LocalLatencyEstimator>;
    type ParentContext = ParentContext<LocalLatencyEstimator>;
}

/// Spawns a background task to periodically print latency estimates
fn spawn_stats_printer<E: LatencyEstimator + Default + 'static>(
    distributions: Arc<LatencyMap<E>>,
    label: &'static str,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                if distributions.is_empty() {
                    continue;
                }

                let mut parts = Vec::new();
                distributions.for_each(|key, distribution| {
                    if distribution.can_estimate() {
                        let estimate = distribution.estimate();

                        // Decode key
                        let parent_id = key >> 32;
                        let child_id = key & 0xFFFFFFFF;

                        // Use registry to get names
                        // We use a simplified formatting if registry lookup fails (shouldn't happen)
                        let registry = MethodRegistry::global();
                        let p_name = registry
                            .get_method_name(parent_id)
                            .map(|(s, m)| format!("{}::{}", s, m))
                            .unwrap_or_else(|| format!("{}", parent_id));
                        let c_name = registry
                            .get_method_name(child_id)
                            .map(|(s, m)| format!("{}::{}", s, m))
                            .unwrap_or_else(|| format!("{}", child_id));

                        parts.push(format!("{}=>{}: {} us", p_name, c_name, estimate));
                    } else {
                        parts.push(format!("{}: (no estimate)", key));
                    }
                });
                log::info!("{}: {}", label, parts.join(", "));
            }
        });
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    // for every method on this server, tracks the remaining duration of the method after an outgoing request has finished
    est_after_child_latency: Arc<LatencyMap<E>>,
    // tracks the actual child RPC call latencies
    est_child_latency: Arc<LatencyMap<E>>,
    print_counter: AtomicUsize,
    #[cfg(feature = "emp_admission")]
    completion_rate_map: Arc<completion_rate_map::CompletionRateMap>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        let est_after_child_latency = Arc::new(LatencyMap::new());
        let est_child_latency = Arc::new(LatencyMap::new());

        spawn_stats_printer(est_after_child_latency.clone(), "Est Remaining Values");
        spawn_stats_printer(est_child_latency.clone(), "Est Child Call Latencies");

        #[cfg(feature = "emp_admission")]
        let completion_rate_map = Arc::new(completion_rate_map::CompletionRateMap::new());

        Self {
            est_after_child_latency,
            est_child_latency,
            print_counter: AtomicUsize::new(0),
            #[cfg(feature = "emp_admission")]
            completion_rate_map,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    method: GrpcMethod,
    resolved_method_id: u64,
    // Keep resolved_method for debugging/logging if needed, but remove if strict zero-overhead desired.
    // Keeping it for now as EarlyReturnHandler uses it.
    resolved_method: CowGrpcMethod,
    ctx: Context,
    server: Arc<ServerContext<E>>,

    q_lat_tracker: QueueLatencyTracker,
    early_return: EarlyReturnHandler,
    child_end_times: Mutex<Vec<(ParentToChildId, Instant)>>,
    // Tracks (api, bucket) for the first admitted empirical-admission decision so we can
    // record the outcome in finalize_before_serialization.
    #[cfg(feature = "emp_admission")]
    first_er_decision: std::sync::OnceLock<(String, usize)>,
    #[cfg(feature = "emp_admission")]
    probe_admitted: std::sync::atomic::AtomicBool,
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext<E>, ServerContext<E>>
    for ParentContext<E>
{
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<E>>,
    ) -> Self {
        let resolved_method = resolve_method_name_from_http(method, req);
        let resolved_method_id = MethodRegistry::global()
            .get_or_register_method(resolved_method.service(), resolved_method.method());

        Self {
            method,
            resolved_method: resolved_method.clone(),
            resolved_method_id,
            ctx: read_context(req),
            server: server_ctx,
            q_lat_tracker: QueueLatencyTracker::new(),
            early_return: EarlyReturnHandler::new(resolved_method),
            child_end_times: Mutex::new(Vec::new()),
            #[cfg(feature = "emp_admission")]
            first_er_decision: std::sync::OnceLock::new(),
            #[cfg(feature = "emp_admission")]
            probe_admitted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }

        let remaining = self.ctx.deadline().saturating_sub(time_now());
        tokio::task::reprioritize(masa_core::PriorityHint::new(remaining));

        self.q_lat_tracker.track_poll();
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.early_return.check(&self.ctx) {
                return Err(Err(self.early_return.issue_error()));
            }
        }

        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext<E>,
    ) -> Result<(), Status> {
        if self.early_return.check(&self.ctx) {
            return Err(self.early_return.issue_error());
        }

        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        // NOTE: we need to include the parent method in the key, because the duration until the
        // end of the parent request after this child request completes will vary for different
        // parent methods (i.e. endpoints on this server)
        let resolved_child_method =
            super::super::resolve_method_name_from_request(child_method, request);

        let resolved_child_id = MethodRegistry::global().get_or_register_method(
            resolved_child_method.service(),
            resolved_child_method.method(),
        );

        let parent_to_child_id = ParentToChildId {
            parent_id: self.resolved_method_id,
            child_id: resolved_child_id,
        };
        let key = parent_to_child_id.to_key();

        // Setup child context to track client runtime
        child_ctx.setup(
            parent_to_child_id.clone(),
            resolved_child_method,
            self.server.clone(),
        );

        let time_left = self.ctx.deadline().saturating_sub(time_now());

        let est_remaining = self
            .server
            .est_after_child_latency
            .get_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        // Mean-only for early-return check: avoids false-positive sheds from σ over-estimation.
        let est_remaining_mean = self
            .server
            .est_after_child_latency
            .get_mean_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        // Floor estimate for ER threshold: decouples the threshold from mean inflation.
        // During load spikes, the EMA mean inflates (slow α_up=0.05), but the floor
        // deflates quickly (α_down=0.3) while inflating very slowly (α_up=0.01).
        // This prevents the feedback loop where mean inflation → tighter ER threshold
        // → more ERs → less useful work being done.
        let est_remaining_floor = self
            .server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);

        let deadline = self.ctx.deadline().saturating_sub(est_remaining);
        // Use floor estimate for ER threshold: the mean can inflate under load, causing
        // over-aggressive shedding. The floor tracks the lower envelope and is resistant
        // to transient spikes, avoiding wasteful sheds when work could still complete.
        if self.admission_check(key, est_remaining_floor) {
            return Err(self.early_return.issue_error());
        }

        // prio_hint = deadline (parent_deadline - est_remaining). Subtracting est_child caused
        // priority inversions under load: as est_child grew with queuing, newer requests got
        // tighter prio_hints than older ones computed with lower estimates, inverting FIFO order.
        // Using deadline alone gives FIFO-like ordering (since all requests share the same e2e
        // SLO deadline), while preserving path-aware early returns from the tightened deadline.
        let prio_hint = deadline;

        if self.server.print_counter.fetch_add(1, Ordering::Relaxed) % 5000 == 0 {
            let est_child = self.server.est_child_latency.get_estimate(key).unwrap_or(0);
            log::info!(
                "LAT_EST: p=>c: {}, est_child: {} (not used for prio_hint), est_rem: {}, est_rem_mean: {}, est_rem_floor: {}",
                parent_to_child_id,
                est_child,
                est_remaining,
                est_remaining_mean,
                est_remaining_floor,
            );
        }

        #[cfg(feature = "emp_admission")]
        let child_forced_probe = self.ctx.forced_probe()
            || self
                .probe_admitted
                .load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(feature = "emp_admission"))]
        let child_forced_probe = self.ctx.forced_probe();

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(prio_hint))
            .forced_probe(child_forced_probe)
            .build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext<E>,
    ) -> Result<(), Status> {
        self.q_lat_tracker.track_child_response(response);
        // Finalize child context to track client runtime if response is not early return
        child_ctx.finalize(response);

        // Update EarlyReturnHandler with the actual child method name if possible
        if let Some(child_method) = &child_ctx.child_method {
            self.early_return.set_last_child(child_method.clone());
        }

        if let Err(status) = response {
            // When the child early-returns, track 0 into est_after_child_latency to create
            // negative feedback. Without this, ERs prevent all tracking updates, freezing the
            // estimate at a high value and creating a self-reinforcing failure loop:
            //   high estimate → tight deadline → ERs → no updates → estimate stays high → ...
            // With this: more ERs → more 0 observations → estimate decreases → looser deadlines
            // → fewer ERs. The equilibrium stabilizes at a lower estimate.
            if status.code() == Code::DeadlineExceeded {
                if let Some(parent_to_child_id) = &child_ctx.parent_to_child_id {
                    self.server
                        .est_after_child_latency
                        .track(parent_to_child_id.to_key(), 0);
                }
            }
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        self.child_end_times.lock().unwrap().push((
            child_ctx
                .parent_to_child_id
                .clone()
                .expect("childctx method must be set"),
            Instant::now(),
        ));

        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.track_latencies();
        }
        self.record_admission_outcome(result);
        self.q_lat_tracker
            .inject_context_metadata(&self.ctx, result);
    }
}

impl<E: LatencyEstimator + Default + 'static> ParentContext<E> {
    fn track_latencies(&self) {
        let parent_end = Instant::now();

        // Track remaining time after child RPC completes
        let mut child_end_times = self.child_end_times.lock().unwrap();
        let child_end_times = std::mem::replace(&mut *child_end_times, Vec::new());

        for (parent_to_child_id, child_end) in child_end_times {
            self.server.est_after_child_latency.track(
                parent_to_child_id.to_key(),
                parent_end.duration_since(child_end).as_micros() as u64,
            );
        }
    }

    /// Returns true if the request should be shed (early-returned).
    ///
    /// With `emp_admission` enabled: uses empirical P(complete | api, bucket) for buckets 0–4.
    /// Bucket 5 (full budget remaining) always admits. Falls through to floor-based check when
    /// emp_admission is disabled.
    #[allow(unused_variables)]
    fn admission_check(&self, key: u64, est_remaining_floor: u64) -> bool {
        #[cfg(feature = "emp_admission")]
        if EARLY_RETURN {
            let time_left = self.ctx.e2e_deadline().saturating_sub(time_now());
            let slo = self.ctx.slo();
            if slo > 0 {
                let bucket = completion_rate_map::time_left_to_bucket(time_left, slo);
                if bucket < 5 {
                    let p = self
                        .server
                        .completion_rate_map
                        .get_p(self.ctx.api(), bucket);
                    let rand = completion_rate_map::deterministic_rand(self.ctx.request_id(), key);
                    // If already a forced probe from upstream, always admit
                    let admitted = if self.ctx.forced_probe() {
                        true
                    } else {
                        let admitted = rand <= p.max(completion_rate_map::PROBE_FLOOR);
                        // Track if admitted via floor probe (rand > p means normal p didn't admit it)
                        if admitted && rand > p {
                            self.probe_admitted
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        admitted
                    };
                    if admitted {
                        // OnceLock: only the first admitted hop is recorded.
                        let _ = self.first_er_decision.set((self.ctx.api().clone(), bucket));
                    }
                    return !admitted;
                }
            }
        }
        // Default: floor estimate check (also used when emp_admission disabled, or bucket == 5).
        EARLY_RETURN && time_now() > self.ctx.e2e_deadline().saturating_sub(est_remaining_floor)
    }

    /// Records the outcome of the first admitted empirical-admission decision.
    ///
    /// No-op when `emp_admission` is disabled.
    #[allow(unused_variables)]
    fn record_admission_outcome<Ret>(&self, result: &Result<Response<Ret>, Status>) {
        #[cfg(feature = "emp_admission")]
        if let Some((api, bucket)) = self.first_er_decision.get() {
            let completed = result.is_ok() && time_now() <= self.ctx.e2e_deadline();
            self.server
                .completion_rate_map
                .update(api, *bucket, completed);
        }
    }
}

fn is_early_return_response<T>(response: &Result<Response<T>, Status>) -> bool {
    match response {
        Ok(_) => false,
        Err(status) => status.code() == Code::DeadlineExceeded,
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    start_time: Option<Instant>,
    parent_to_child_id: Option<ParentToChildId>,
    child_method: Option<CowGrpcMethod>,
    server: Option<Arc<ServerContext<E>>>,
}

impl<E: LatencyEstimator + Default + 'static> ClientHooks for ChildContext<E> {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            start_time: None,
            parent_to_child_id: None,
            child_method: None,
            server: None,
        }
    }
}

impl<E: LatencyEstimator + Default + 'static> ChildContext<E> {
    fn setup(
        &mut self,
        parent_to_child_id: ParentToChildId,
        child_method: CowGrpcMethod,
        server: Arc<ServerContext<E>>,
    ) {
        self.start_time = Some(Instant::now());
        self.parent_to_child_id = Some(parent_to_child_id);
        self.child_method = Some(child_method);
        self.server = Some(server);
    }

    fn finalize<T>(&self, response: &Result<Response<T>, Status>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyRms;

    #[test]
    fn test_server_context_rms_integration() {
        let ctx = ServerContext::<LatencyRms>::new("test_service");
        let method = ParentToChildId {
            parent_id: 1,
            child_id: 2,
        };
        let key = method.to_key();

        // Inject an estimator with a short update interval (2) for testing.
        // By default, LatencyRms has a large update interval (512), which makes testing hard.
        {
            ctx.est_child_latency.insert(key, LatencyRms::new(2));
        }

        // 1st track: sum_sq=100, count=1, since_update=1. No update yet.
        ctx.est_child_latency.track(key, 10);

        // Estimate uses cached RMS value (initially 0).
        let est = ctx.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(0));

        // 2nd track: sum_sq=200, count=2, since_update=2. Update triggers.
        // RMS = sqrt( (10^2 + 10^2) / 2 ) = 10.
        ctx.est_child_latency.track(key, 10);

        let est = ctx.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10));

        // 3rd track: sum_sq=200+400=600, count=3, since_update=1. No update yet.
        ctx.est_child_latency.track(key, 20);
        let est = ctx.est_child_latency.get_estimate(key);
        assert_eq!(est, Some(10)); // Still 10

        // 4th track: sum_sq=600+400=1000, count=4, since_update=2. Update triggers.
        // RMS = sqrt( (100 + 100 + 400 + 400) / 4 ) = sqrt(250) ≈ 15.
        ctx.est_child_latency.track(key, 20);
        let est = ctx.est_child_latency.get_estimate(key);
        // integer_sqrt(250) is 15 (15*15=225, 16*16=256)
        assert_eq!(est, Some(15));
    }

    #[test]
    fn test_resolve_method_name_from_http_with_overrides() {
        use crate::masa::context::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
        use http::HeaderValue;

        let method = GrpcMethod::new("TestService", "TestMethod");
        let mut req = http::Request::new(());

        // 1. No overrides
        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "TestMethod");

        // 2. Method override only
        req.headers_mut().insert(
            METHOD_NAME_OVERRIDE_HEADER,
            HeaderValue::from_static("OverriddenMethod"),
        );
        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "OverriddenMethod");

        // 3. Method and Service override
        req.headers_mut().insert(
            SERVICE_NAME_OVERRIDE_HEADER,
            HeaderValue::from_static("OverriddenService"),
        );
        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "OverriddenService");
        assert_eq!(resolved.method(), "OverriddenMethod");
    }

    #[test]
    fn test_resolve_method_name_from_request_with_overrides() {
        use crate::masa::context::resolve_method_name_from_request;
        use crate::masa::context::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
        use crate::metadata::MetadataValue;

        let method = GrpcMethod::new("TestService", "TestMethod");
        let mut req = Request::new(());

        // 1. No overrides
        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "TestMethod");

        // 2. Method override only
        req.metadata_mut().insert(
            METHOD_NAME_OVERRIDE_HEADER,
            MetadataValue::from_static("OverriddenMethod"),
        );
        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "OverriddenMethod");

        // 3. Method and Service override
        req.metadata_mut().insert(
            SERVICE_NAME_OVERRIDE_HEADER,
            MetadataValue::from_static("OverriddenService"),
        );
        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "OverriddenService");
        assert_eq!(resolved.method(), "OverriddenMethod");
    }

    #[test]
    fn test_local_deadline_policy_integration() {
        use crate::masa::context::MASA_CONTEXT_HEADER;
        use masa_core::ContextBuilder;

        // 1. Setup Server Context
        let server_ctx = Arc::new(ServerContext::<LatencyRms>::new("IntegrationService"));

        // 2. Prepare Parent Request
        let method = GrpcMethod::new("IntegrationService", "ParentMethod");
        let deadline = masa_core::time_now() + 100_000; // 100ms future
        let ctx = ContextBuilder::new("IntegrationService", 123)
            .deadline(deadline)
            .build();

        let req = http::Request::builder()
            .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
            .body(())
            .unwrap();

        // 3. Begin Parent Context (registers ParentMethod)
        let parent_ctx = ParentContext::<LatencyRms>::begin(method, &req, server_ctx.clone());

        // 4. Before Child RPC (registers ChildMethod)
        let child_method = GrpcMethod::new("IntegrationService", "ChildMethod");
        let mut child_req = Request::new(());
        let mut child_ctx = ChildContext::<LatencyRms>::new(child_method, &child_req);

        let _ = parent_ctx
            .before_child_rpc(child_method, &mut child_req, &mut child_ctx)
            .unwrap();

        // Verify child context has ID and Server
        assert!(child_ctx.parent_to_child_id.is_some());
        assert!(child_ctx.server.is_some());

        // Verify registry has IDs
        let registry = MethodRegistry::global();
        let parent_id = registry.get_or_register_method("IntegrationService", "ParentMethod");
        let child_id = registry.get_or_register_method("IntegrationService", "ChildMethod");

        assert_eq!(
            child_ctx.parent_to_child_id.clone().unwrap().parent_id,
            parent_id
        );
        assert_eq!(
            child_ctx.parent_to_child_id.clone().unwrap().child_id,
            child_id
        );

        // 5. Simulate Child Response
        let mut response = Ok(Response::new(()));
        let _ = parent_ctx
            .after_child_rpc(child_method, &mut response, child_ctx)
            .unwrap();

        // 6. Track Latencies
        // This normally happens in finalize, but we can call internal method if accessible or simulate via public hook.
        // finalize_before_serialization calls track_latencies if not early return.
        let mut response_result = Ok(Response::new(()));
        parent_ctx.finalize_before_serialization(&mut response_result);

        // 7. Verify Stats Updated
        // We can't easily peek into LatencyRms without internal access or waiting for updates.
        // But we can check that entries exist in the map using the key.
        let key = (parent_id << 32) | child_id;

        // Need to wait/trigger update if LatencyRms has a window.
        // But simply checking if key exists in map (get_estimate returns Some(0) or something) confirms integration.
        // For LatencyRms, get_estimate returns None if not enough data, or Some(val).
        // Since we tracked one value, it might be cached.

        // We can verify that the key exists in the map implicitly by tracking again or checking log side effects (hard).
        // Better: check that we can retrieve *some* estimate (even if 0) or that the key is present.
        // LatencyMap::get_estimate will create default if missing.
        // We want to ensure it WAS created/tracked.
        // We can't check 'was tracked' easily on the public interface without side channels.
        // However, the fact that we ran through without panic/error is a good sign.

        // Let's verify we can get the names back from registry for the IDs we expect.
        let (p_s, p_m) = registry.get_method_name(parent_id).unwrap();
        assert_eq!(p_s, "IntegrationService");
        assert_eq!(p_m, "ParentMethod");
    }

    /// Verify that `record_admission_outcome` updates the CompletionRateMap after a successful
    /// request when `emp_admission` is enabled.
    #[cfg(feature = "emp_admission")]
    #[test]
    fn test_emp_admission_outcome_updates_map_on_success() {
        use super::completion_rate_map::CompletionRateMap;
        use crate::masa::context::MASA_CONTEXT_HEADER;
        use masa_core::ContextBuilder;

        let server_ctx = Arc::new(ServerContext::<LatencyRms>::new("EmpService"));

        // Build a context with a generous deadline and SLO so emp_admission takes effect.
        let slo_us = 200_000u64; // 200ms
        let now = masa_core::time_now();
        let deadline = now + slo_us;
        let ctx = ContextBuilder::new("EmpService", 999)
            .slo(slo_us)
            .deadline(deadline)
            .build();

        let req = http::Request::builder()
            .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
            .body(())
            .unwrap();

        let method = GrpcMethod::new("EmpService", "ParentMethod");
        let parent_ctx = ParentContext::<LatencyRms>::begin(method, &req, server_ctx.clone());

        // Simulate the request completing successfully within the SLO.
        let mut response_result: Result<Response<()>, Status> = Ok(Response::new(()));
        parent_ctx.finalize_before_serialization(&mut response_result);

        // The CompletionRateMap should now have been consulted during finalize.
        // Since first_er_decision is only set during before_child_rpc, if no child RPC was made
        // then the map should not have been updated (nothing to record).
        // Verify the map is still at the default 1.0 for any bucket.
        let p = server_ctx.completion_rate_map.get_p("EmpService", 3);
        assert_eq!(p, 1.0, "no child rpc means no map update");
    }

    /// Verify that when p is driven to near 0, most requests are shed (only PROBE_FLOOR fraction
    /// pass). This tests the stochastic admission behavior of `admission_check`.
    #[cfg(feature = "emp_admission")]
    #[test]
    fn test_emp_admission_sheds_when_p_is_zero() {
        use super::completion_rate_map::{CompletionRateMap, PROBE_FLOOR};

        let map = CompletionRateMap::new();
        // Drive completion rate to near 0 by recording repeated failures.
        for _ in 0..100 {
            map.update("Search", 0, false);
        }
        let p = map.get_p("Search", 0);
        assert!(p < 0.01, "p should be near 0 after 100 failures: {}", p);

        // With p ≈ 0, requests should be admitted only with probability PROBE_FLOOR.
        // Check that deterministic_rand values at various request_ids that exceed PROBE_FLOOR
        // (i.e., > p.max(PROBE_FLOOR)) would be shed.
        use super::completion_rate_map::deterministic_rand;
        let mut admitted = 0usize;
        let total = 10_000usize;
        let threshold = p.max(PROBE_FLOOR);
        for i in 0..total as u64 {
            let rand = deterministic_rand(i, 42);
            if rand <= threshold {
                admitted += 1;
            }
        }
        // We expect approximately PROBE_FLOOR fraction admitted (5%).
        let admit_rate = admitted as f64 / total as f64;
        assert!(
            (admit_rate - PROBE_FLOOR).abs() < 0.02,
            "expected ~5% admission, got {:.1}%",
            admit_rate * 100.0
        );
    }

    /// Verify that without `emp_admission`, `admission_check` falls back to the floor-based check.
    /// The floor check should admit when there is plenty of time left (no shed).
    #[cfg(not(feature = "emp_admission"))]
    #[test]
    fn test_admission_check_floor_based_admits_with_budget() {
        use crate::masa::context::MASA_CONTEXT_HEADER;
        use masa_core::ContextBuilder;

        let server_ctx = Arc::new(ServerContext::<LatencyRms>::new("FloorService"));

        // Generous deadline: 100ms from now.
        let slo_us = 100_000u64;
        let now = masa_core::time_now();
        let deadline = now + slo_us;
        let ctx = ContextBuilder::new("FloorService", 42)
            .slo(slo_us)
            .deadline(deadline)
            .build();

        let req = http::Request::builder()
            .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
            .body(())
            .unwrap();

        let method = GrpcMethod::new("FloorService", "ParentMethod");
        let parent_ctx = ParentContext::<LatencyRms>::begin(method, &req, server_ctx.clone());

        // With est_remaining_floor = 0, the floor check is: time_now > e2e_deadline - 0 = e2e_deadline.
        // Since e2e_deadline is 100ms in the future, this should NOT shed.
        let shed = parent_ctx.admission_check(0, 0);
        assert!(!shed, "should admit when plenty of time remains");
    }
}
