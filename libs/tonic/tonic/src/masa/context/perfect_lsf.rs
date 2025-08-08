use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::Arc;

use super::super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use masa::{time_now, Context};

#[derive(Debug)]
/// This policy always sets the deadline of each request as
///   d = (start + SLO) - rem // slack
/// where start is the point in time when the end-to-end request from the client was sent out
/// and `rem` is the sum of all latencies at the remainig hops of the
/// end-to-end request.
/// This policy expects that `rem` is known in advance and is placed in the request header.
pub struct PerfectLSF;

impl PrioritySelector for PerfectLSF {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
pub struct ServerContext {}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}

#[derive(Debug)]
pub struct ParentContext {
    ctx: Context,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        _method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            ctx: read_context(req),
        }
    }

    fn before_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        log::warn!("using LSF!!!");
        let remaining_execution_time = request.metadata().get("remaining_execution_time").unwrap();
        let remaining_execution_time: u64 =
            remaining_execution_time.to_str().unwrap().parse().unwrap();
        let deadline = self.ctx.start_at() + self.ctx.slo();
        let sub = time_now() + remaining_execution_time;
        let slack = if deadline >= sub { deadline - sub } else { 0 };

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.test_id(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.request_class(),
            self.ctx.start_at(),
            slack,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
