use std::sync::Arc;

use crate::{GrpcMethod, Request};

use super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};

#[derive(Debug)]
pub struct NoopPrioritySelector;

impl PrioritySelector for NoopPrioritySelector {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

/// A noop implementation of `ParentHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {}

/// A simple implementation of `ClientHooks`.
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
        Self {}
    }
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}

impl ServerHooks for ServerContext {
    /// Construct a NoopServerContext.
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}
