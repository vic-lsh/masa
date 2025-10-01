use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use crate::body::BoxBody;
use crate::Response;
use masa::Context;

#[derive(Debug)]
/// This policy always sets the deadline of each request as
///   d = start + SLO
/// where start is the point in time when the original request from the client was sent out
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct QueueGlobal;

impl PrioritySelector for QueueGlobal {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ServerContext {}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}

#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    ctx: Context,
    q_lat: AtomicU64,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            ctx: read_context(req),
            q_lat: AtomicU64::new(0),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        if queue_latency > 0 {
            self.q_lat.fetch_add(queue_latency, Ordering::AcqRel);
        }
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        // TODO: early return logic
        let deadline = self.ctx.deadline();

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.test_id(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.request_class(),
            self.ctx.start_at(),
            deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

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
                        self.q_lat.fetch_add(parsed, Ordering::AcqRel);
                    }
                }
            }
        }
        Ok(())
    }

    // expect frontend method, all other method are going send back their latency trace
    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let res_header = _response.headers_mut();
        let total = self.q_lat.load(Ordering::Acquire).to_string();
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
