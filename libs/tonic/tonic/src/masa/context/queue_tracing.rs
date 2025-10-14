//! Initial hook implementation that traces a request's compute, IO, and queueing latencies.

use crate::body::BoxBody;
use crate::Response;
use crate::{GrpcMethod, Request, Status};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use super::super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct QueueTracing;

impl PrioritySelector for QueueTracing {
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
    q_lat: AtomicU64,
    max_child: AtomicU64,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        _req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            q_lat: AtomicU64::new(0),
            max_child: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        if queue_latency > 0 {
            self.q_lat.fetch_add(queue_latency, Ordering::AcqRel);
        }
        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Ok(resp) = response {
            if let Some(value) = resp
                .metadata()
                .get("x-queue-latency")
                .or_else(|| resp.metadata().get("X-Queue-Latency"))
            {
                if let Ok(v) = value.to_str() {
                    if let Ok(parsed) = v.parse::<u64>() {
                        let prev = self.max_child.load(Ordering::Acquire);
                        if parsed > prev {
                            self.max_child.store(parsed, Ordering::Release);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // expect frontend method, all other method are going send back their latency trace
    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let res_header = _response.headers_mut();
        let final_q_lat = self.q_lat.load(Ordering::Acquire) + self.max_child.load(Ordering::Acquire);
        let total = final_q_lat.to_string();
        if let Ok(header_val) = http::HeaderValue::from_str(&total) {
            // HTTP/2 metadata is lower-case; rely on hyper to canonicalize.
            res_header.insert("x-queue-latency", header_val);
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
