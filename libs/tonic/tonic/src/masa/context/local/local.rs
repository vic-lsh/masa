use crate::{
    masa::context::{read_context, METHOD_NAME_OVERRIDE_HEADER},
    Code, GrpcMethod, Request, Response, Status,
};
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc, Mutex, RwLock},
    task::Poll,
    time::{Duration, Instant},
};

use super::super::common::{EarlyReturnHandler, QueueLatencyTracker};
use super::super::{
    resolve_method_name, ClientHooks, MasaHooks, MasaRequestExt, ParentHooks, ServerHooks,
};
use super::{get_estimate, track_method_latency, PERCENTILE};
use masa_core::{
    time_now, Context, ContextBuilder, LatencyDistribution, LatencyEstimator, PriorityHint,
    EARLY_RETURN,
};
use std::sync::atomic::AtomicUsize;

/// Type alias for the latency estimator used in the local deadline policy.
/// Change this to use a different estimator (e.g., `LatencyRms`).
pub(crate) type LocalLatencyEstimator = LatencyDistribution;

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
    distributions: Arc<RwLock<HashMap<String, E>>>,
    label: &'static str,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                let distributions_read = distributions.read().unwrap();
                if distributions_read.is_empty() {
                    continue;
                }

                let mut parts = Vec::new();
                for (endpoint, distribution) in distributions_read.iter() {
                    if distribution.can_estimate() {
                        let estimate = distribution.estimate(PERCENTILE);
                        parts.push(format!("{}: {} us", endpoint, estimate));
                    } else {
                        parts.push(format!("{}: (no estimate)", endpoint));
                    }
                }
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
    est_after_child_latency: Arc<RwLock<HashMap<String, E>>>,
    // tracks the actual child RPC call latencies
    est_child_latency: Arc<RwLock<HashMap<String, E>>>,
    print_counter: AtomicUsize,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        let est_after_child_latency = Arc::new(RwLock::new(HashMap::<String, E>::new()));
        let est_child_latency = Arc::new(RwLock::new(HashMap::<String, E>::new()));

        spawn_stats_printer(est_after_child_latency.clone(), "Est Remaining Values");
        spawn_stats_printer(est_child_latency.clone(), "Est Child Call Latencies");

        Self {
            est_after_child_latency,
            est_child_latency,
            print_counter: AtomicUsize::new(0),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    method: GrpcMethod,
    resolved_method: String,
    ctx: Context,
    server: Arc<ServerContext<E>>,

    q_lat_tracker: QueueLatencyTracker,
    early_return: EarlyReturnHandler,
    child_end_times: Mutex<Vec<(String, Instant)>>,
    // Map from child_method.id() to resolved child method name
    // resolved_child_methods: Mutex<HashMap<MethodId, String>>,
}

/// Resolve the method name from HTTP request headers, checking for override header.
fn resolve_method_name_from_http<B>(method: GrpcMethod, req: &http::Request<B>) -> String {
    if let Some(header_value) = req.headers().get(METHOD_NAME_OVERRIDE_HEADER) {
        if let Ok(method_name) = header_value.to_str() {
            return method_name.to_string();
        }
    }
    method.id().to_string()
}

/// Resolve the method name from Request metadata, checking for override header.
fn resolve_method_name_from_request<T>(method: GrpcMethod, request: &Request<T>) -> String {
    if let Some(header_value) = request.metadata().get(METHOD_NAME_OVERRIDE_HEADER) {
        if let Ok(method_name) = header_value.to_str() {
            return method_name.to_string();
        }
    }
    method.id().to_string()
}

/// Concatenate parent and child method names.
fn parent_to_child_identifier(parent: &str, child: &str) -> String {
    format!("{}=>{}", parent, child)
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
        Self {
            method,
            resolved_method,
            ctx: read_context(req),
            server: server_ctx,
            q_lat_tracker: QueueLatencyTracker::new(),
            early_return: EarlyReturnHandler::new(
                method.service(),
                resolve_method_name(method, req),
            ),
            child_end_times: Mutex::new(Vec::new()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }

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
        let resolved_child_method = resolve_method_name_from_request(child_method, request);

        let parent_to_child_id =
            parent_to_child_identifier(&self.resolved_method, &resolved_child_method);

        // Setup child context to track client runtime
        child_ctx.setup(parent_to_child_id.clone(), self.server.clone());

        let est_remaining = get_estimate(
            &*self.server.est_after_child_latency,
            parent_to_child_id.clone(),
        )
        .unwrap_or(0);

        let deadline = self.ctx.deadline() - est_remaining;
        if EARLY_RETURN && time_now() > deadline {
            return Err(self.early_return.issue_error());
        }

        let est_child =
            get_estimate(&*self.server.est_child_latency, parent_to_child_id.clone()).unwrap_or(0);

        // this encodes the slack: parent deadline - est child latency - est remaining
        let prio_hint = deadline - est_child;

        if self.server.print_counter.fetch_add(1, Ordering::Relaxed) % 5000 == 0 {
            log::info!(
                "LAT_EST: p=>c: {}, est_child: {}, est_rem: {}",
                parent_to_child_id,
                est_child,
                est_remaining
            );
        }

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(prio_hint))
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

        if let Some(parent_to_child_id) = &child_ctx.parent_to_child_id {
            if let Some((_, child)) = parent_to_child_id.split_once("=>") {
                self.early_return.set_last_child(child.to_string());
            }
        }

        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        self.child_end_times.lock().unwrap().push((
            child_ctx
                .parent_to_child_id
                .expect("childctx method must be set"),
            Instant::now(),
        ));

        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.track_latencies();
        }
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
            track_method_latency(
                &*self.server.est_after_child_latency,
                parent_to_child_id,
                parent_end.duration_since(child_end).as_micros() as u64,
            );
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
    parent_to_child_id: Option<String>,
    server: Option<Arc<ServerContext<E>>>,
}

impl<E: LatencyEstimator + Default + 'static> ClientHooks for ChildContext<E> {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            start_time: None,
            parent_to_child_id: None,
            server: None,
        }
    }
}

impl<E: LatencyEstimator + Default + 'static> ChildContext<E> {
    fn setup(&mut self, parent_to_child_id: String, server: Arc<ServerContext<E>>) {
        self.start_time = Some(Instant::now());
        self.parent_to_child_id = Some(parent_to_child_id);
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
            track_method_latency(
                &*server.est_child_latency,
                parent_to_child_id.clone(),
                client_runtime,
            );
        }
    }
}
