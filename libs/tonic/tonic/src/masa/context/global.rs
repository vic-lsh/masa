use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::Arc;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::MasaRequestExt;
use masa_core::{Context, ContextBuilder};

#[derive(Debug)]
/// This policy always sets the deadline of each request as
///   d = start + SLO
/// where start is the point in time when the original request from the client was sent out
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct Global;

impl MasaHooks for Global {
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
        // TODO: early return logic
        let deadline = self.ctx.deadline();
        let prio_hint = self.ctx.prio_hint();

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(prio_hint)
            .build();
        request.set_masa_context(&child_recv_ctx);

        Ok(())
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
