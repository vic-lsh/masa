use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    task::Poll,
    time::Instant,
};

use tonic_masa::{
    Context, FutureGraphTracker, LatencyDistribution, LatencyTracker, LocalGraphTracker, MethodId,
};

use crate::{body::BoxBody, masa::mock_graph, GrpcMethod, Request, Response, Status};

use super::{ClientHooks, ParentHooks, PrioritySelector, ServerHooks};

#[derive(Debug)]
// TODO: document
pub struct LocalDeadlineIndirect;

impl PrioritySelector for LocalDeadlineIndirect {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

const DISTRIBUTION_CAPACITY: usize = 1024;
const PERCENTILE: usize = 50;

/// A simple implementation of `ParentHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParentContext {
    method: GrpcMethod,
    ctx: Context,
    server_ctx: Arc<ServerContext>,

    start: Instant,
    duration_tracker: LatencyTracker,
    estimated_duration: u64,

    child_ctxs: Arc<Mutex<Vec<ChildContext>>>,
}

/// A simple implementation of `ClientHooks`.
#[derive(Debug, Clone)]
pub struct ChildContext {
    method: GrpcMethod,
    duration_tracker: LatencyTracker,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    parent_distributions: RwLock<HashMap<MethodId, LatencyDistribution>>,
    child_distributions: RwLock<HashMap<MethodId, HashMap<MethodId, LatencyDistribution>>>,
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        // log::info!("parent_ctx, begin, method: {:?}", method.id());
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let ctx = Context::from_json(ctx_str);
        let mut duration_tracker = LatencyTracker::NotStarted;
        duration_tracker.start();
        let start = Instant::now();
        let has_method = server_ctx
            .parent_distributions
            .read()
            .unwrap()
            .contains_key(method.id());
        if !has_method {
            // TODO: where to get capacity from?
            server_ctx.parent_distributions.write().unwrap().insert(
                method.id(),
                LatencyDistribution::new(method.id().to_string(), 1024),
            );
        }
        let estimated_duration = server_ctx
            .parent_distributions
            .read()
            .unwrap()
            .get(method.id())
            .unwrap()
            .estimate(PERCENTILE);
        Self {
            method,
            ctx,
            server_ctx,
            start,
            duration_tracker,
            estimated_duration,
            child_ctxs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        // TODO: early return logic?
        let elapsed = Instant::now().duration_since(self.start);
        let estimate_child = 0;
        self.server_ctx.child_distributions.
        let estimate_remaining = self.estimated_duration - ;

        let deadline = self.ctx.deadline() - estimate_remaining;

        Ok(Context::new(
            self.ctx.api().clone(),
            self.ctx.test_id(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.request_class(),
            self.ctx.start_at(),
            deadline,
        ))
    }

    fn after_child_rpc<T>(
        &self,
        child_rpc_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Result<(), Status> {
        // log::info!(
        //     "parent_ctx, after_child_rpc, method: {:?}",
        //     child_rpc_method.id()
        // );

        if let Err(status) = response {
            return Err(status.clone());
        }

        let latency = child_ctx
            .duration_tracker
            .get_latency()
            .unwrap()
            .as_micros() as u64;
        let mut graph = self
            .server_ctx
            .future_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .write()
            .unwrap();
        graph.track_present_span(child_rpc_method.id(), latency);

        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        let mut child_ctx = child_ctx.clone();
        child_ctx.future_tracker.start();
        self.child_ctxs.lock().unwrap().push(child_ctx);

        Ok(())
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        // log::info!("parent_ctx, before_poll, method: {:?}", self.method.id());
        // if self.polled.fetch_add(1, Ordering::Relaxed) == 0 {
        //     if self.check_early_return() {
        //         return Some(Err(self.issue_early_return()));
        //     }
        // }

        let now = time_now();
        self.last_before_poll.store(now, Ordering::Release);
        let last_after_poll = self.last_after_poll.load(Ordering::Acquire);
        if last_after_poll != 0 {
            let io_lat = now - last_after_poll;
            self.io_lat.fetch_add(io_lat, Ordering::Release);
        }

        if self.check_early_return() {
            return Err(Err(self.issue_early_return()));
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        let now = time_now();
        self.last_after_poll.store(now, Ordering::Release);
        let last_before_poll = self.last_before_poll.load(Ordering::Acquire);
        assert!(last_before_poll != 0);
        let compute_lat = now - last_before_poll;
        self.compute_lat.fetch_add(compute_lat, Ordering::Release);

        match poll {
            Poll::Pending => {
                if self.check_early_return() {
                    return Err(Err(self.issue_early_return()));
                }
            }
            Poll::Ready(_) => {}
        };

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        self.duration_tracker.record_latency();
        let duration = self.duration_tracker.get_latency().unwrap();
    }
}

impl ClientHooks for ChildContext {
    fn new<T>(method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            method,
            duration_tracker: LatencyTracker::NotStarted,
            future_tracker: LatencyTracker::NotStarted,
        }
    }

    fn before_send<T>(&mut self, _request: &mut Request<T>) {
        // log::info!("child_ctx, before_send, method: {:?}", self.method.id());
        self.duration_tracker.start();
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        // log::info!(
        //     "child_ctx, after_recv, method: {:?}, elapsed: {} us",
        //     self.method.id(),
        //     self.track_latency.get_latency().unwrap().as_micros(),
        // );
        self.duration_tracker.record_latency();
    }
}

impl ServerHooks for ServerContext {
    /// Construct a SimpleServerContext.
    fn new(service_name: &'static str) -> Self {
        // [NOTE] Hierarachy:
        // - Application
        //  - Service
        //   - Method
        //    - Span (Method / Compute)

        let global_graph = mock_graph::hotel::get_global_graph(service_name.to_string());

        // [NOTE] Ideally, trackers should be Hashmap<MethodId, LatencyTracker>.
        // But for now, two trackers are tracking the same method ID separately.
        // Namely, each local graph has its own tracker.

        let local_graphs = global_graph.local_graphs().clone();
        let local_graph_trackers = local_graphs
            .iter()
            .map(|(path, local_graph)| {
                let local_graph = LocalGraphTracker::from(local_graph.clone());
                (*path, RwLock::new(local_graph))
            })
            .collect();

        let local_graphs = global_graph.local_graphs().clone();
        let future_graph_trackers = local_graphs
            .iter()
            .map(|(path, local_graph)| {
                let future_graph = FutureGraphTracker::from(local_graph.clone());
                (*path, RwLock::new(future_graph))
            })
            .collect();

        let num_early_returns = Arc::new(AtomicUsize::new(0));
        // let num_early_returns_clone = num_early_returns.clone();
        // tokio::spawn(async move {
        //     let secs = 10;
        //     let mut prev = 0;
        //     loop {
        //         tokio::time::sleep(Duration::from_secs(secs)).await;
        //         let curr = num_early_returns_clone.load(Ordering::Relaxed);
        //         log::info!(
        //             "num early returns: {}, diff in {} secs: {}",
        //             curr,
        //             secs,
        //             curr - prev
        //         );
        //         prev = curr;
        //     }
        // });

        log::warn!(
            "SimpleServerContext, service: {:?}, local_graphs: {:?}",
            service_name,
            local_graphs
        );

        Self {
            service_name,
            num_early_returns,
            local_graphs,
            local_graph_trackers,
            future_graph_trackers,
        }
    }
}
