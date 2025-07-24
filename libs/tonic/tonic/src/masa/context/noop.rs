use std::{sync::Arc, time::Instant};

use masa::time_now;

use super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use crate::{GrpcMethod, Request};

#[derive(Debug)]
pub struct NoopPrioritySelector;

impl PrioritySelector for NoopPrioritySelector {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {}

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

    fn before_send<T>(&mut self, request: &mut Request<T>) {
        request
            .metadata_mut()
            .insert("sent_at", time_now().to_string().parse().unwrap());
    }
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {}
    }
}
