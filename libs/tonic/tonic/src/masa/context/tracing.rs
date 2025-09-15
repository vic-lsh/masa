//! Initial hook implementation that traces a request's compute, IO, and queueing latencies.

use crate::body::BoxBody;
use crate::Response;
use crate::metadata::MetadataMap;
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

// Extract latency traces from the response headers
fn extract_latency_traces(
    metadata: &MetadataMap,
) -> Option<Vec<LatencyTrace>> {
    let header_value = metadata.get("X-Latency-Traces").expect("missing X-Latency-Traces header").to_str().unwrap();
    let traces: Vec<LatencyTrace> = serde_json::from_str(header_value).ok()?;
    Some(traces)
}


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
    method: GrpcMethod,

    num_polled: AtomicUsize,
    start_exec: Instant,
    last_before_poll: AtomicU64,
    compute_latency: AtomicU64,
    latency_traces: Mutex<Vec<LatencyTrace>>,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method: _method,
            num_polled: AtomicUsize::new(0),
            start_exec: Instant::now(),
            last_before_poll: AtomicU64::new(0),
            compute_latency: AtomicU64::new(0),
            latency_traces: Mutex::new(Vec::new()),
        }
    }

    fn after_child_rpc<T>(
            &self,
            method: GrpcMethod,
            response: &mut Result<Response<T>, Status>,
            child_ctx: ChildContext,
        ) -> Result<(), Status> {

        // Obtain the vector of latency traces from the child context
        if let Ok(res)= response {
            if let Some(child_traces) = extract_latency_traces(res.metadata()) {
                let mut traces = self.latency_traces.lock().unwrap();
                traces.extend(child_traces);
            }
        }
        Ok(())
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

    // expect frontend method, all other method are going send back their latency trace
    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let lat_trace = self.get_latency_trace();
        {
            let mut traces = self.latency_traces.lock().unwrap();
            traces.push(lat_trace);
            // Print size of the traces vector
            println!(
                "Finalizing. Total traces to send back: {}",
                traces.len()
            );
            let res_header = _response.headers_mut();
            let traces_json = serde_json::to_string(&*traces).unwrap();
            let header_val = http::HeaderValue::from_str(&traces_json).unwrap();
            res_header.insert("X-Latency-Traces", header_val);
       }
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
            method_id: self.method.id().to_string(),
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
