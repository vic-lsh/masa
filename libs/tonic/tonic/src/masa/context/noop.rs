use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use crate::masa::context::read_context;
use crate::{GrpcMethod, Request};

use crate::body::BoxBody;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crate::Response;
use crate::Status;

use masa::{time_now};

#[derive(Debug)]
pub struct NoopPrioritySelector;

impl PrioritySelector for NoopPrioritySelector {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {
    method: GrpcMethod,

    start_exec: Instant,
    last_before_poll: Mutex<Option<Instant>>,
    compute_latency: AtomicU64,

}

#[derive(Debug, Clone)]
pub struct ChildContext {}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        _req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method: _method,
            start_exec: Instant::now(),
            last_before_poll: Mutex::new(None),
            compute_latency: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        self.last_before_poll.lock().unwrap().replace(Instant::now());
        Ok(())
    }

    fn after_poll<Ret>(
            &self,
            poll: &std::task::Poll<Result<Response<Ret>, Status>>,
        ) -> Result<(), Result<Response<Ret>, Status>> {
        let mut last_before_poll_guard = self.last_before_poll.lock().unwrap();
        if let Some(start_time) = last_before_poll_guard.take() {
            let compute_latency = start_time.elapsed().as_micros() as u64;
            self.compute_latency.fetch_add(compute_latency, Ordering::AcqRel);
        }
        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        // read the timer from the header
        let e2e_latency = self.start_exec.elapsed().as_micros() as u64;
        let compute_latency = self.compute_latency.load(Ordering::Acquire);
        // obtain the queue lat from header
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        let io_latency = e2e_latency.saturating_sub(compute_latency).saturating_sub(queue_latency);

        let latency_str = format!("{},{},{},{}", e2e_latency, compute_latency, io_latency, queue_latency);
        // insert the latency info to header
        let res_header = _response.headers_mut();
        let header_val = http::HeaderValue::from_str(&latency_str).unwrap();
        res_header.insert("X-Request-Latency", header_val);
    }
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}
