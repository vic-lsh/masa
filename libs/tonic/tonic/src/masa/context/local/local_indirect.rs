use crate::{body::BoxBody, masa::context::read_context, GrpcMethod, Request, Response, Status};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use super::{estimate_method_latency, track_method_latency};
use masa::{Context, LatencyDistribution, LatencyEstimator, LatencyTracker};

#[derive(Debug)]
/// Same as LocalDeadlineDirect, but e_rem
/// is computed as
///   e_rem = e_p - t_now - e_curr
/// where
///   - e_p is an estimate of the total duration of the parent request, computed by sampling from
///   the distribution of observed durations
///   - t_now is the amount of time the parent request has already executed for
///   - e_curr is an estimate of the total duration of the child request, computed analogous to e_p
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlineIndirect;

impl MasaHooks for LocalDeadlineIndirect {
    type ServerContext = ServerContext<LatencyDistribution>;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext<LatencyDistribution>;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext<E: LatencyEstimator + Default + 'static = LatencyDistribution> {
    // tracks latency distribution for each method provided by this server
    parent_distributions: RwLock<HashMap<String, E>>,
    // tracks latency distribution for each method called by this server
    child_distributions: RwLock<HashMap<String, E>>,
}

impl<E: LatencyEstimator + Default + 'static> ServerHooks for ServerContext<E> {
    fn new(_service_name: &'static str) -> Self {
        Self {
            parent_distributions: RwLock::new(HashMap::new()),
            child_distributions: RwLock::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext<E: LatencyEstimator + Default + 'static = LatencyDistribution> {
    method: GrpcMethod,
    ctx: Context,
    server: Arc<ServerContext<E>>,

    start: Instant,
    estimated_duration: Option<u64>,
}

impl<E: LatencyEstimator + Default + 'static> ParentHooks<ChildContext, ServerContext<E>> for ParentContext<E> {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<E>>,
    ) -> Self {
        let start = Instant::now();
        let estimated_duration =
            estimate_method_latency(&server_ctx.parent_distributions, method.id().to_string());
        Self {
            method,
            ctx: read_context(req),
            server: server_ctx,
            estimated_duration,
            start,
        }
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        // TODO: early return logic
        let elapsed = Instant::now().duration_since(self.start).as_micros() as u64;
        let estimate_child = estimate_method_latency(
            &self.server.child_distributions,
            child_method.id().to_string(),
        );
        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        let estimate_remaining = match (self.estimated_duration, estimate_child) {
            (Some(e_p), Some(e_c)) => {
                let elapsed_after_child = elapsed + e_c;
                if e_p >= elapsed_after_child {
                    e_p - elapsed_after_child
                } else {
                    log::error!(
                        "remaining duration estimated as negative: e_p = {}, t_now = {}, e_c = {}",
                        e_p,
                        elapsed,
                        e_c
                    );
                    0
                }
            }
            _ => 0,
        };
        let deadline = self.ctx.deadline() - estimate_remaining;

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.request_id(),
            self.ctx.slo(),
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
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        if let Err(status) = response {
            return Err(status.clone());
        }

        // track child duration
        let duration = child_ctx
            .duration_tracker
            .get_latency()
            .unwrap()
            .as_micros() as u64;
        track_method_latency(
            &self.server.child_distributions,
            child_method.id().to_string(),
            duration,
        );

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        // track latency of this request
        let duration = self.start.elapsed().as_micros() as u64;
        track_method_latency(
            &self.server.parent_distributions,
            self.method.id().to_string(),
            duration,
        );
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext {
    duration_tracker: LatencyTracker,
}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            duration_tracker: LatencyTracker::NotStarted,
        }
    }

    fn before_send<T>(&mut self, _request: &mut Request<T>) {
        self.duration_tracker.start();
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        self.duration_tracker.record_latency();
    }
}
