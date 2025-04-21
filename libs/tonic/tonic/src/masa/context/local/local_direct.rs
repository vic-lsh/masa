use crate::{body::BoxBody, masa::context::read_context, GrpcMethod, Request, Response, Status};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

use super::super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use super::{estimate_method_latency, track_method_latency};
use tonic_masa::{Context, LatencyDistribution, MethodId};

#[derive(Debug)]
// TODO: document
pub struct LocalDeadlineDirect;

impl PrioritySelector for LocalDeadlineDirect {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
pub struct ServerContext {
    // for every method on this server, tracks the remaining duration of the method after an outgoing request has finished
    child_distributions: RwLock<HashMap<(MethodId, MethodId), LatencyDistribution>>,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            child_distributions: RwLock::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
pub struct ParentContext {
    method: GrpcMethod,
    ctx: Context,
    server: Arc<ServerContext>,

    child_end_times: Mutex<Vec<(MethodId, Instant)>>,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method,
            ctx: read_context(req),
            server: server_ctx,
            child_end_times: Mutex::new(Vec::new()),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        // TODO: early return logic?
        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        let estimate_remaining = match estimate_method_latency(
            &self.server.child_distributions,
            (method.id(), child_ctx.method.id()),
        ) {
            Some(x) => x,
            None => 0,
        };
        let deadline = self.ctx.deadline() - estimate_remaining;

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
        child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Err(status) = response {
            return Err(status.clone());
        }

        self.child_end_times
            .lock()
            .unwrap()
            .push((child_method.id(), Instant::now()));

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        let parent_end = Instant::now();
        // track remaining time after each child
        for (child_method, child_end) in self.child_end_times.lock().unwrap().iter() {
            track_method_latency(
                &self.server.child_distributions,
                (self.method.id(), child_method),
                parent_end.duration_since(*child_end).as_millis() as u64,
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {
    method: GrpcMethod,
}

impl ClientHooks for ChildContext {
    fn new<T>(method: GrpcMethod, _request: &Request<T>) -> Self {
        Self { method }
    }
}
