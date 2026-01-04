//! Initial hook implementation that traces a request's compute, IO, and queueing latencies.

use crate::body::BoxBody;
use crate::Response;
use crate::{GrpcMethod, Request, Status};
use std::sync::Mutex;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use masa::{time_now, FutureSpan};

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct Tracing;

impl MasaHooks for Tracing {
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
    method: GrpcMethod,

    start_exec: Instant,
    last_before_poll: AtomicU64,
    last_after_poll: AtomicU64,
    first_rpc_stamp: AtomicU64,
    second_rpc_stamp: AtomicU64,
    latency_traces: Mutex<Vec<FutureSpan>>,
    is_rpc: std::sync::atomic::AtomicBool,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        _req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method: _method,
            start_exec: Instant::now(),
            last_before_poll: AtomicU64::new(0),
            last_after_poll: AtomicU64::new(0),
            first_rpc_stamp: AtomicU64::new(0),
            second_rpc_stamp: AtomicU64::new(0),
            latency_traces: Mutex::new(Vec::new()),
            is_rpc: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn before_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        _request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        self.is_rpc.store(true, Ordering::Release);
        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        let block_latency = self
            .second_rpc_stamp
            .swap(0, Ordering::AcqRel)
            .saturating_sub(self.first_rpc_stamp.swap(0, Ordering::AcqRel))
            .saturating_sub(queue_latency);
        self.latency_traces
            .lock()
            .unwrap()
            .push(FutureSpan::ChildBlock(block_latency));
        self.latency_traces
            .lock()
            .unwrap()
            .push(FutureSpan::Queueing(queue_latency));
        self.is_rpc.store(false, Ordering::Release);
        Ok(())
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        let now = time_now();
        self.last_before_poll.store(now, Ordering::Release);
        if self.is_rpc.load(Ordering::Acquire) {
            // if we are in an rpc, we do not count the queueing time
            self.second_rpc_stamp.store(now, Ordering::Release);
            return Ok(());
        }

        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        {
            let mut traces = self.latency_traces.lock().unwrap();
            let last_after_poll = self.last_after_poll.swap(0, Ordering::AcqRel);
            if last_after_poll != 0 {
                let block_latency = now
                    .saturating_sub(last_after_poll)
                    .saturating_sub(queue_latency);
                traces.push(FutureSpan::LocalBlock(block_latency));
            }
            traces.push(FutureSpan::Queueing(queue_latency));
        }

        Ok(())
    }
    fn after_poll<Ret>(
        &self,
        _poll: &std::task::Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        let now = time_now();

        if self.is_rpc.load(Ordering::Acquire) {
            // if we are in an rpc, we do not count the polling time
            if self.first_rpc_stamp.load(Ordering::Acquire) == 0 {
                self.first_rpc_stamp.store(now, Ordering::Release);

                self.last_after_poll.store(now, Ordering::Release);
                let last_before_poll = self.last_before_poll.load(Ordering::Acquire);
                let compute_latency = now - last_before_poll;
                self.latency_traces
                    .lock()
                    .unwrap()
                    .push(FutureSpan::Compute(compute_latency));
            }
            return Ok(());
        }

        self.last_after_poll.store(now, Ordering::Release);
        let last_before_poll = self.last_before_poll.load(Ordering::Acquire);
        let compute_latency = now - last_before_poll;
        self.latency_traces
            .lock()
            .unwrap()
            .push(FutureSpan::Compute(compute_latency));
        Ok(())
    }

    // expect frontend method, all other method are going send back their latency trace
    fn finalize_after_serialization(&self, _response: &mut http::Response<BoxBody>) {
        let res_header = _response.headers_mut();
        let traces_json = serde_json::to_string(&*self.latency_traces.lock().unwrap()).unwrap();
        let header_val = http::HeaderValue::from_str(&traces_json).unwrap();
        res_header.insert("X-Latency-Traces", header_val);
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
