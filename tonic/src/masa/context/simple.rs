use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock,
    },
    task::Poll,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tonic_masa::{
    Context, LocalGraph, LocalGraphTracker, MethodId, FIFO, FIFO_INFRA, PRIO_GLOBAL,
    PRIO_GLOBAL_EARLY, PRIO_LOCAL, PRIO_LOCAL_EARLY,
};

use crate::{body::BoxBody, masa::mock_graph, Code, GrpcMethod, Request, Response, Status};

use super::{ClientStubHooks, RequestHandlerHooks, ServerContext, ServerHooks};

#[inline]
fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

/// A simple implementation of `RequestHandlerHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SimpleParentContext {
    method: GrpcMethod,
    ctx: Context,
    server_ctx: Arc<ServerContext>,

    will_early_return: AtomicBool,
    polled: AtomicUsize,
    request_start: Instant,
}

/// A simple implementation of `ClientStubHooks`.
#[derive(Debug)]
pub struct SimpleChildContext {
    _method: GrpcMethod,
    latency_tracker: LatencyTracker,
}

#[derive(Debug)]
enum LatencyTracker {
    NotStarted,
    Started(Instant),
    Finished(Duration),
}

impl LatencyTracker {
    fn start(&mut self) {
        *self = match self {
            Self::NotStarted => Self::Started(Instant::now()),
            _ => panic!("Cannot start tracking latency twice"),
        }
    }

    fn record_latency(&mut self) {
        *self = match self {
            Self::Started(inst) => Self::Finished(inst.elapsed()),
            _ => panic!("Cannot record latency if the tracker hasn't started"),
        }
    }

    fn get_latency(&self) -> Option<Duration> {
        match self {
            Self::Finished(lat) => Some(*lat),
            _ => None,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct SimpleServerContext {
    service_name: &'static str,
    num_early_returns: Arc<AtomicUsize>,
    local_graphs: HashMap<MethodId, LocalGraph>,
    local_graph_trackers: HashMap<MethodId, RwLock<LocalGraphTracker>>,
}

impl SimpleParentContext {
    #[inline]
    fn check_early_return(&self) -> bool {
        // if self.method.id() == "/frontend.Frontend/HandleSearch"
        if PRIO_GLOBAL_EARLY || PRIO_LOCAL_EARLY {
            if self.will_early_return.load(Ordering::Relaxed) {
                return true;
            }

            let now = time_now();
            let should_early_return = now >= self.ctx.deadline();

            if should_early_return {
                // `check_early_return` may be invoked at multiple lifecycle hooks.
                //
                // this will only be read/written on one thread, so we can use the
                // weakest ordering guarantees.
                // it is an atomic because the ParentContext type needs to be Sync:
                // see the docs for RequestHandlerHooks for why.
                if self
                    .will_early_return
                    .compare_exchange_weak(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    self.server_ctx
                        .num_early_returns
                        .fetch_add(1, Ordering::Relaxed);
                }
            }

            return should_early_return;
        } else {
            return false;
        }
    }

    #[inline]
    fn issue_early_return<T>(&self) -> Result<Response<T>, Status> {
        Err(Status::new(
            Code::DeadlineExceeded,
            format!("EarlyReturn/{}", self.method.id()),
        ))
    }
}

impl RequestHandlerHooks<SimpleChildContext, SimpleServerContext> for SimpleParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<SimpleServerContext>,
    ) -> Self {
        // log::info!("parent_ctx, begin, method: {:?}", method.id());
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let ctx = Context::from_json(ctx_str);
        Self {
            method,
            ctx,
            server_ctx,
            will_early_return: AtomicBool::new(false),
            polled: AtomicUsize::new(0),
            request_start: Instant::now(),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        _child_send_ctx: &mut SimpleChildContext,
    ) -> Option<Status> {
        // log::info!("parent_ctx, before_child_rpc, method: {:?}", method.id());
        if self.check_early_return() {
            return Some(Status::new(Code::DeadlineExceeded, self.method.id()));
        }

        let graph = self
            .server_ctx
            .local_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .read()
            .unwrap();
        let deadline;
        let latest_exec;
        if FIFO || FIFO_INFRA || PRIO_GLOBAL || PRIO_GLOBAL_EARLY {
            deadline = self.ctx.deadline();
            latest_exec = self.ctx.latest_exec();
        } else if PRIO_LOCAL || PRIO_LOCAL_EARLY {
            // [TODO:LD] Return two values at one time.
            deadline = self.ctx.deadline() - graph.estimate_suffix_deadline(&method.id());
            latest_exec = self.ctx.deadline() - graph.estimate_suffix_latest_exec(&method.id());
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
            latest_exec,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        None
    }

    fn after_child_rpc<T>(
        &self,
        child_rpc_method: GrpcMethod,
        resp: &mut Result<Response<T>, Status>,
        child_ctx: SimpleChildContext,
    ) -> Option<Status> {
        // log::info!(
        //     "parent_ctx, after_child_rpc, method: {:?}",
        //     child_rpc_method.id()
        // );

        if let Err(status) = resp {
            return Some(status.clone());
        }

        let latency_us = child_ctx.latency_tracker.get_latency().unwrap().as_micros();
        let mut graph = self
            .server_ctx
            .local_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .write()
            .unwrap();
        graph.track_span(&child_rpc_method.id(), latency_us as u64);

        if self.check_early_return() {
            return Some(Status::new(Code::DeadlineExceeded, self.method.id()));
        }
        None
    }

    fn before_poll<Ret>(&self) -> Option<Result<Response<Ret>, Status>> {
        // log::info!("parent_ctx, before_poll, method: {:?}", self.method.id());
        if self.polled.fetch_add(1, Ordering::Relaxed) == 0 {
            if self.check_early_return() {
                return Some(self.issue_early_return());
            }
        }
        None
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Option<Result<Response<Ret>, Status>> {
        match poll {
            Poll::Pending => {
                // if self.check_early_return() {
                //     return Some(self.issue_early_return());
                // }
            }
            Poll::Ready(_) => {}
        };
        None
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        // log::info!(
        //     "parent_ctx, finalize, method: {:?}, polled: {} times, elapsed: {} us",
        //     self.method.id(),
        //     self.polled.load(Ordering::Relaxed),
        //     self.request_start.elapsed().as_micros()
        // );
    }
}

impl ClientStubHooks for SimpleChildContext {
    fn new<T>(method: GrpcMethod, _req: &Request<T>) -> Self {
        Self {
            _method: method,
            latency_tracker: LatencyTracker::NotStarted,
        }
    }

    fn before_send<T>(&mut self, _req: &mut Request<T>) {
        // log::info!("child_ctx, before_send, method: {:?}", self.method.id());
        self.latency_tracker.start();
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        // log::info!(
        //     "child_ctx, after_recv, method: {:?}, elapsed: {} us",
        //     self.method.id(),
        //     self.track_latency.get_latency().unwrap().as_micros(),
        // );
        self.latency_tracker.record_latency();
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
        }
    }
}
