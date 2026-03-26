use std::sync::Arc;

use super::{ClientHooks, Hooks, ParentHooks, ServerHooks};
use crate::{GrpcMethod, Request};

/// No-op hooks implementation used when no scheduling features are enabled.
#[derive(Debug)]
#[allow(dead_code)]
pub struct NoopHooks;

impl Hooks for NoopHooks {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

/// No-op parent context.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {}

/// No-op child context.
#[derive(Debug, Clone)]
pub struct ChildContext {}

/// No-op server context.
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
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}
