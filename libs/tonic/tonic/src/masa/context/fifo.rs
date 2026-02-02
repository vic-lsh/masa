use crate::{masa::context::read_context, GrpcMethod, Request, Status};
use std::sync::Arc;
use std::task::Poll;

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::common::EarlyReturnHandler;
use super::{resolve_method_name, METHOD_NAME_OVERRIDE_HEADER};
use crate::Response;
use masa::{Context, ContextBuilder, PriorityHint};

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

/// Resolve the method name from Request metadata, checking for override header.
fn resolve_method_name_from_request<T>(method: GrpcMethod, request: &Request<T>) -> String {
    if let Some(header_value) = request.metadata().get(METHOD_NAME_OVERRIDE_HEADER) {
        if let Ok(method_name) = header_value.to_str() {
            return method_name.to_string();
        }
    }
    method.id().to_string()
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
        if self.early_return.check(&self.ctx) {
            return Err(Err(self.early_return.issue_error()));
        }
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.early_return.check(&self.ctx) {
            return Err(self.early_return.issue_error());
        }

        let child_method_name = resolve_method_name_from_request(child_method, request);
        child_ctx.set_method_name(child_method_name);

        let deadline = self.ctx.deadline();

        let child_recv_ctx = ContextBuilder::from(&self.ctx)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(deadline))
            .build();
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _child_method: GrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Some(child) = child_ctx.child_method_name {
            self.early_return.set_last_child(child);
        }
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        match poll {
            Poll::Pending => {
                if self.early_return.check(&self.ctx) {
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
pub struct ChildContext {
    pub child_method_name: Option<String>,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
        }
    }
}

impl ChildContext {
    pub fn set_method_name(&mut self, name: String) {
        self.child_method_name = Some(name);
    }
}
