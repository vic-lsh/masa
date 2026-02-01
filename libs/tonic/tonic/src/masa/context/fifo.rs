use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::Arc;
use std::task::Poll;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::common::EarlyReturnHandler;
use super::resolve_method_name;
use crate::Response;
use masa::{Context, ContextBuilder, PriorityHint, EARLY_RETURN};

#[derive(Debug)]
/// FIFO policy with optional early return support.
/// Requests are served in first-in-first-out order.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct Fifo;

impl MasaHooks for Fifo {
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
    early_return: EarlyReturnHandler,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        _server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            ctx: read_context(req),
            early_return: EarlyReturnHandler::new(
                method.service(),
                resolve_method_name(method, req),
            ),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.early_return.check(&self.ctx, EARLY_RETURN) {
            return Err(Err(self.early_return.issue_error()));
        }
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.early_return.check(&self.ctx, EARLY_RETURN) {
            return Err(self.early_return.issue_error());
        }

        let deadline = self.ctx.deadline();

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(deadline))
            .build();
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        match poll {
            Poll::Pending => {
                if self.early_return.check(&self.ctx, EARLY_RETURN) {
                    return Err(Err(self.early_return.issue_error()));
                }
            }
            Poll::Ready(_) => {}
        };

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
