use std::sync::Arc;
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
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct NoopPrioritySelector;

impl PrioritySelector for NoopPrioritySelector {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext {}

#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct ChildContext {}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext {}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        _req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method: _method,
            num_polled: AtomicUsize::new(0),
            start_exec: Instant::now(),
            last_before_poll: AtomicU64::new(0),
            compute_latency: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        let now = time_now();
        self.last_before_poll.store(now, Ordering::Release);
        
        Ok(())
    }

    fn after_poll<Ret>(
            &self,
            poll: &std::task::Poll<Result<Response<Ret>, Status>>,
        ) -> Result<(), Result<Response<Ret>, Status>> {
        let now = time_now();
        let last_before_poll = self.last_before_poll.load(Ordering::Acquire);
        assert!(last_before_poll != 0);
        let compute_latency = now - last_before_poll;
        self.compute_latency.fetch_add(compute_latency, Ordering::AcqRel);

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        // read the timer from the header
        let e2e_latency = self.start_exec.elapsed().as_micros() as u64;
        let compute_latency = self.compute_latency.load(Ordering::Acquire);
        // obtain the queue lat from header
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        let io_latency = e2e_latency.saturating_sub(compute_latency).saturating_sub(queue_latency);

        // Get the task of the request
        log::info!("Request {:?} completed. E2E latency: {}, Compute latency: {}, IO latency: {}, Queue latency: {}",
            self.method.id(), e2e_latency, compute_latency, io_latency, queue_latency);
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
