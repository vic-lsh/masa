use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

use super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};
use tonic_masa::{Context, LatencyDistribution, MethodId};

#[derive(Debug)]
// TODO: document
pub struct LocalDeadlineDirect;

impl PrioritySelector for LocalDeadlineDirect {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}
// TODO: tweak these values
const DISTRIBUTION_CAPACITY: usize = 1024;
const MIN_DISTRIBUTION_SIZE: usize = 500;
const PERCENTILE: usize = 50;

#[derive(Debug)]
pub struct ServerContext {
    child_distributions: RwLock<HashMap<MethodId, LatencyDistribution>>,
}

impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            child_distributions: RwLock::new(HashMap::new()),
        }
    }
}

fn estimate_method_duration(
    map: &RwLock<HashMap<MethodId, LatencyDistribution>>,
    method: MethodId,
) -> Option<u64> {
    let has_method = map.read().unwrap().contains_key(method);
    if !has_method {
        // TODO: where to get capacity from?
        map.write().unwrap().insert(
            method,
            LatencyDistribution::new(method.to_string(), DISTRIBUTION_CAPACITY),
        );
    } else {
        let lock = map.read().unwrap();
        let distribution = lock.get(method).unwrap();

        if distribution.len() > MIN_DISTRIBUTION_SIZE {
            return Some(distribution.estimate(PERCENTILE));
        }
    }

    None
}

fn track_method_duration(
    map: &RwLock<HashMap<MethodId, LatencyDistribution>>,
    method: MethodId,
    duration: u64,
) {
    map.write()
        .unwrap()
        .get_mut(method)
        .unwrap()
        .track(duration);
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
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let ctx = Context::from_json(ctx_str);
        Self {
            method,
            ctx,
            server: server_ctx,
            child_end_times: Mutex::new(Vec::new()),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        // TODO: early return logic?
        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        let estimate_remaining =
            match estimate_method_duration(&self.server.child_distributions, method.id()) {
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
        for (method, child_end) in self.child_end_times.lock().unwrap().iter() {
            track_method_duration(
                &self.server.child_distributions,
                method,
                parent_end.duration_since(*child_end).as_millis() as u64,
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
