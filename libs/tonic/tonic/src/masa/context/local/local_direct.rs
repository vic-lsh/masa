//! This module is deprecated.

use crate::{body::BoxBody, masa::context::read_context, GrpcMethod, Request, Response, Status};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, RwLock,
    },
    task::Poll,
    time::{Duration, Instant},
};

use super::super::super::{ClientHooks, MasaHooks, MasaRequestExt, ParentHooks, ServerHooks};
use super::super::common::EarlyReturnHandler;
use super::super::resolve_method_name_from_http;
use super::{estimate_method_latency, track_method_latency};
use masa_core::{Context, ContextBuilder, LatencyDistribution, LatencyEstimator, MethodId};

static LAST_PRINT_TIME: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

#[derive(Debug)]
/// This policy computes the deadline d of a child request as
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlineDirect;

impl MasaHooks for LocalDeadlineDirect {
    type ServerContext = ServerContext<LatencyDistribution>;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext<LatencyDistribution>;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LatencyDistribution> {
    // for every method on this server, tracks the remaining duration of the method after an outgoing request has finished
    child_distributions: RwLock<HashMap<String, E>>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        Self {
            child_distributions: RwLock::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LatencyDistribution> {
    method: GrpcMethod,
    ctx: Context,
    q_lat: AtomicU64,
    server: Arc<ServerContext<E>>,
    early_return: EarlyReturnHandler,
    child_end_times: Mutex<Vec<(MethodId, Instant)>>,
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext, ServerContext<E>>
    for ParentContext<E>
{
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<E>>,
    ) -> Self {
        Self {
            method,
            ctx: read_context(req),
            server: server_ctx,
            early_return: EarlyReturnHandler::new(resolve_method_name_from_http(method, req)),
            child_end_times: Mutex::new(Vec::new()),
            q_lat: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        if queue_latency > 0 {
            self.q_lat.fetch_add(queue_latency, Ordering::AcqRel);
        }

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
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.early_return.check(&self.ctx) {
            return Err(self.early_return.issue_error());
        }

        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        // NOTE: we need to include the parent method in the key, because the duration until the
        // end of the parent request after this child request completes will vary for different
        // parent methods (i.e. endpoints on this server)
        let estimate_remaining = estimate_method_latency(
            &self.server.child_distributions,
            format!("{}/{}", self.method.id(), child_method.id()),
        )
        .unwrap_or(0);

        // Print estimate_remaining every 5 seconds
        {
            let last_print = LAST_PRINT_TIME.get_or_init(|| Mutex::new(None));
            let mut last_print_guard = last_print.lock().unwrap();
            let now = Instant::now();
            let should_print = last_print_guard
                .map(|last| now.duration_since(last) >= Duration::from_secs(5))
                .unwrap_or(true);

            if should_print {
                println!(
                    "estimate_remaining: {}, before child {}",
                    estimate_remaining,
                    child_method.id()
                );
                *last_print_guard = Some(now);
            }
        }

        // NOTE(vic): could we have passed the deadline at this point?
        let deadline = self.ctx.deadline() - estimate_remaining;
        let prio_hint = self.ctx.prio_hint();

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(prio_hint)
            .build();
        request.set_masa_context(&child_recv_ctx);

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

        self.child_end_times
            .lock()
            .unwrap()
            .push((child_method.id().into(), Instant::now()));

        if let Ok(resp) = response {
            if let Some(value) = resp
                .metadata()
                .get("x-queue-latency")
                .or_else(|| resp.metadata().get("X-Queue-Latency"))
            {
                if let Ok(v) = value.to_str() {
                    if let Ok(parsed) = v.parse::<u64>() {
                        self.q_lat.fetch_add(parsed, Ordering::AcqRel);
                    }
                }
            }
        }

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let parent_end = Instant::now();
        let res_header = _response.headers_mut();
        let total = self.q_lat.load(Ordering::Acquire).to_string();
        if let Ok(header_val) = http::HeaderValue::from_str(&total) {
            // HTTP/2 metadata is lower-case; rely on hyper to canonicalize.
            res_header.insert("x-queue-latency", header_val);
        }
        // track remaining time after each child
        for (child_method, child_end) in self.child_end_times.lock().unwrap().iter() {
            // TODO: The LatencyDistribution instances will regularly sort their data. Should this
            // work be done asynchronously?
            track_method_latency(
                &self.server.child_distributions,
                format!("{}/{}", self.method.id(), child_method),
                parent_end.duration_since(*child_end).as_micros() as u64,
            );
        }
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
