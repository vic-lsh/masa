use crate::{
    body::BoxBody,
    masa::context::{read_context, METHOD_NAME_OVERRIDE_HEADER},
    Code, GrpcMethod, Request, Response, Status,
};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    task::Poll,
    time::{Duration, Instant},
};

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::{estimate_method_latency, track_method_latency, PERCENTILE};
use masa::{time_now, Context, LatencyEstimator, LatencyRms, MethodId, EARLY_RETURN};

/// Type alias for the latency estimator used in the local deadline policy.
/// Change this to use a different estimator (e.g., `LatencyRms`).
pub(crate) type LocalLatencyEstimator = LatencyRms;

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
    type ChildContext = ChildContext;
    type ParentContext = ParentContext<LocalLatencyEstimator>;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LocalLatencyEstimator> {
    // for every method on this server, tracks the remaining duration of the method after an outgoing request has finished
    child_distributions: Arc<RwLock<HashMap<String, E>>>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        let distributions = Arc::new(RwLock::new(HashMap::<String, E>::new()));

        // Spawn a background task to print estimated remaining values every second
        let distributions_clone = distributions.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;

                    let distributions_read = distributions_clone.read().unwrap();
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
                    println!("Est Remaining Values: {}", parts.join(", "));
                }
            });
        }

        Self {
            child_distributions: distributions,
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

    will_early_return: AtomicBool,
    child_end_times: Mutex<Vec<(String, Instant)>>,
    // Map from child_method.id() to resolved child method name
    resolved_child_methods: Mutex<HashMap<MethodId, String>>,
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

impl<E: LatencyEstimator + Default + 'static> ParentContext<E> {
    #[inline]
    fn check_early_return(&self) -> bool {
        if EARLY_RETURN {
            self.check_early_return_impl()
        } else {
            false
        }
    }

    fn check_early_return_impl(&self) -> bool {
        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= self.ctx.deadline();

        if should_early_return {
            if self
                .will_early_return
                .compare_exchange_weak(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {}
        }

        should_early_return
    }

    #[inline]
    fn issue_early_return(&self) -> Status {
        Status::new(Code::DeadlineExceeded, format!("/EarlyReturn"))
    }
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext, ServerContext<E>>
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
            will_early_return: AtomicBool::new(false),
            child_end_times: Mutex::new(Vec::new()),
            resolved_child_methods: Mutex::new(HashMap::new()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.check_early_return() {
            return Err(Err(self.issue_early_return()));
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.check_early_return() {
                return Err(Err(self.issue_early_return()));
            }
        }

        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        // NOTE: we need to include the parent method in the key, because the duration until the
        // end of the parent request after this child request completes will vary for different
        // parent methods (i.e. endpoints on this server)
        let resolved_child_method = resolve_method_name_from_request(child_method, request);

        // Store the resolved child method name for use in after_child_rpc
        self.resolved_child_methods
            .lock()
            .unwrap()
            .insert(child_method.id().into(), resolved_child_method.clone());

        let estimate_remaining = estimate_method_latency(
            &*self.server.child_distributions,
            format!("{} -> {}", self.resolved_method, resolved_child_method),
        )
        .unwrap_or(0);

        let deadline = self.ctx.deadline() - estimate_remaining;
        if EARLY_RETURN && time_now() > deadline {
            return Err(self.issue_early_return());
        }

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.start_at(),
            deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        // Retrieve the resolved child method name that was stored in before_child_rpc
        let method_id: MethodId = Cow::Borrowed(child_method.id());
        let resolved_child_method = self
            .resolved_child_methods
            .lock()
            .unwrap()
            .get(&method_id)
            .cloned()
            .unwrap_or_else(|| child_method.id().to_string());

        self.child_end_times
            .lock()
            .unwrap()
            .push((resolved_child_method, Instant::now()));

        Ok(())
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.track_latencies();
        }
    }

    fn finalize_after_serialization(&self, _response: &mut http::Response<BoxBody>) {}
}

impl<E: LatencyEstimator + Default + 'static> ParentContext<E> {
    fn track_latencies(&self) {
        let parent_end = Instant::now();
        for (child_method, child_end) in self.child_end_times.lock().unwrap().iter() {
            track_method_latency(
                &*self.server.child_distributions,
                format!("{} -> {}", self.resolved_method, child_method),
                parent_end.duration_since(*child_end).as_micros() as u64,
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
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
