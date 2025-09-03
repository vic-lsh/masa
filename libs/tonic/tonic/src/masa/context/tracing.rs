//! Initial hook implementation that traces a request's compute, IO, and queueing latencies.

use crate::body::BoxBody;
use crate::Response;
use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::Mutex;
use std::{
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use super::super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use masa::{time_now, Context, LatencyTrace};

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct Tracing;

impl PrioritySelector for Tracing {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext {}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    // NOTE: this mutex should never be contended.
    // We only access the ctx in the `finalize()` hook, which should only
    // be invoked by one thread, and when all other accesses to ParentContext
    // should have been dropped.
    ctx: Mutex<Context>,
    method: GrpcMethod,

    num_polled: AtomicUsize,
    start_exec: Instant,
    last_before_poll: AtomicU64,
    compute_latency: AtomicU64,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method: _method,
            ctx: Mutex::new(read_context(req)),
            num_polled: AtomicUsize::new(0),
            start_exec: Instant::now(),
            last_before_poll: AtomicU64::new(0),
            compute_latency: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.last_before_poll.store(time_now(), Ordering::Release);

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        _poll: &std::task::Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        let last_before_poll = self.last_before_poll.load(Ordering::Acquire);
        assert!(last_before_poll != 0);
        let compute_latency = time_now() - last_before_poll;
        self.compute_latency
            .fetch_add(compute_latency, Ordering::AcqRel);
        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        self.ctx
            .lock()
            .expect("taking ctx lock should succeed")
            .record_trace(self.method.id(), self.get_latency_trace());
    }
}

impl ParentContext {
    fn get_latency_trace(&self) -> LatencyTrace {
        let e2e_latency_us = self.start_exec.elapsed().as_micros() as u64;
        let compute_latency_us = self.compute_latency.load(Ordering::Acquire);
        let queue_latency_us = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        let io_latency_us = e2e_latency_us
            .saturating_sub(compute_latency_us)
            .saturating_sub(queue_latency_us);

        LatencyTrace {
            e2e_latency_us,
            compute_latency_us,
            queue_latency_us,
            io_latency_us,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
