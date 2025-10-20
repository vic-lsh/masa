use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    task::Poll,
    time::Instant,
};

use masa::{
    time_now, Context, FutureGraphTracker, LatencyTracker, LocalGraph, LocalGraphTracker, MethodId,
    FIFO, PRIO_GLOBAL, PRIO_LOCAL,
};

use crate::{body::BoxBody, masa::mock_graph, Code, GrpcMethod, Request, Response, Status};

use super::{read_context, ClientHooks, ParentHooks, PrioritySelector, ServerHooks};

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct SimplePrioritySelector;

impl PrioritySelector for SimplePrioritySelector {
    type ServerContext = SimpleServerContext;
    type ChildContext = SimpleChildContext;
    type ParentContext = SimpleParentContext;
}

/// A simple implementation of `ParentHooks`.
#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct SimpleParentContext {
    method: GrpcMethod,
    ctx: Context,
    server_ctx: Arc<SimpleServerContext>,

    will_early_return: AtomicBool,
    num_polled: AtomicUsize,
    start_exec: Instant,
    last_before_poll: AtomicU64,
    last_after_poll: AtomicU64,
    compute_lat: AtomicU64,
    io_lat: AtomicU64,

    child_ctxs: Arc<Mutex<Vec<SimpleChildContext>>>,
}

/// A simple implementation of `ClientHooks`.
#[derive(Debug, Clone)]
#[allow(unreachable_pub)]
pub struct SimpleChildContext {
    method: GrpcMethod,
    present_tracker: LatencyTracker,
    future_tracker: LatencyTracker,
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct SimpleServerContext {
    service_name: &'static str,
    num_early_returns: Arc<AtomicUsize>,
    local_graphs: HashMap<MethodId, LocalGraph>,
    local_graph_trackers: HashMap<MethodId, RwLock<LocalGraphTracker>>,
    future_graph_trackers: HashMap<MethodId, RwLock<FutureGraphTracker>>,
}

impl SimpleParentContext {
    #[inline]
    fn check_early_return(&self) -> bool {
        // TODO: add back early return impl
        return false;
    }

    #[inline]
    fn issue_early_return(&self) -> Status {
        Status::new(
            Code::DeadlineExceeded,
            format!("/EarlyReturn{}", self.method.id()),
        )
    }
}

impl ParentHooks<SimpleChildContext, SimpleServerContext> for SimpleParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<SimpleServerContext>,
    ) -> Self {
        Self {
            method,
            ctx: read_context(req),
            server_ctx,
            will_early_return: AtomicBool::new(false),
            num_polled: AtomicUsize::new(0),
            start_exec: Instant::now(),
            last_before_poll: AtomicU64::new(0),
            last_after_poll: AtomicU64::new(0),
            compute_lat: AtomicU64::new(0),
            io_lat: AtomicU64::new(0),
            child_ctxs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut SimpleChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        let graph = self
            .server_ctx
            .future_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .read()
            .unwrap();
        let deadline;
        if FIFO || PRIO_GLOBAL {
            deadline = self.ctx.deadline();
        } else if PRIO_LOCAL {
            deadline = self.ctx.deadline()
                - graph.estimate_future(method.id())
                - graph.estimate_present(method.id());
        } else {
            panic!("Unimplemented policy");
        }

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
        child_rpc_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: SimpleChildContext,
    ) -> Result<(), Status> {
        // log::info!(
        //     "parent_ctx, after_child_rpc, method: {:?}",
        //     child_rpc_method.id()
        // );

        if let Err(status) = response {
            return Err(status.clone());
        }

        let latency = child_ctx.present_tracker.get_latency().unwrap().as_micros() as u64;
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
        let check_early_return = self.check_early_return();
        log::debug!(
            "finalize, ctx: {:?}, method: {:?}, check_early_return: {}, compute_lat: {}, io_lat: {}, total_lat: {}",
            self as *const _,
            self.method.id(),
            check_early_return,
            self.compute_lat.load(Ordering::Acquire),
            self.io_lat.load(Ordering::Acquire),
            self.start_exec.elapsed().as_micros()
        );

        if check_early_return {
            return;
        }
        log::debug!(
            "finalize, ctx: {:?}, method: {:?}, child_ctxs: {}, elapsed: {} us",
            self as *const _,
            self.method.id(),
            self.child_ctxs.lock().unwrap().len(),
            self.start_exec.elapsed().as_micros()
        );

        for child_ctx in self.child_ctxs.lock().unwrap().iter_mut() {
            child_ctx.future_tracker.record_latency();
            let present_latency =
                child_ctx.present_tracker.get_latency().unwrap().as_micros() as u64;
            let future_latency = child_ctx.future_tracker.get_latency().unwrap().as_micros() as u64;
            log::info!(
                "track_future_span, ctx: {:?}, child_rpc_method: {:?}, present_latency: {} us, future_latency: {} us",
                self as *const _,
                child_ctx.method.id(),
                present_latency,
                future_latency
            );

            let mut graph = self
                .server_ctx
                .future_graph_trackers
                .get(&self.method.id())
                .unwrap()
                .write()
                .unwrap();
            graph.track_future_span(child_ctx.method.id(), future_latency);
        }
    }
}

impl ClientHooks for SimpleChildContext {
    fn new<T>(method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {
            method,
            present_tracker: LatencyTracker::NotStarted,
            future_tracker: LatencyTracker::NotStarted,
        }
    }

    fn before_send<T>(&mut self, _request: &mut Request<T>) {
        // log::info!("child_ctx, before_send, method: {:?}", self.method.id());
        self.present_tracker.start();
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        // log::info!(
        //     "child_ctx, after_recv, method: {:?}, elapsed: {} us",
        //     self.method.id(),
        //     self.track_latency.get_latency().unwrap().as_micros(),
        // );
        self.present_tracker.record_latency();
    }
}

impl ServerHooks for SimpleServerContext {
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

        // log::warn!(
        //     "SimpleServerContext, service: {:?}, local_graphs: {:?}",
        //     service_name,
        //     local_graphs
        // );

        Self {
            service_name,
            num_early_returns,
            local_graphs,
            local_graph_trackers,
            future_graph_trackers,
        }
    }
}
