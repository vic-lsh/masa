use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    task::Poll,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tonic_masa::{
    Context, LocalGraph, LocalGraphTracker, MethodId, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_GLOBAL_TWO,
    PRIO_LOCAL, PRIO_LOCAL_TWO,
};

use crate::{body::BoxBody, masa::mock_graph, Code, GrpcMethod, Request, Response, Status};

use super::{ChildContext, ClientStubHooks, RequestHandlerHooks, ServerContext};

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

    //polled: AtomicUsize,
    request_start: Instant,
}

/// A simple implementation of `ClientStubHooks`.
#[derive(Debug)]
pub struct SimpleChildContext {
    method: GrpcMethod,
    track_latency: LatencyTracker,
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
        // if PRIO_GLOBAL_TWO || PRIO_LOCAL_TWO {
        //if self.method.id() != "/frontend.Frontend/HandleSearch" {
        //if self.method.id() == "/rate.Rate/HandleGetRates" {
        let now = time_now();
        let check = now >= self.ctx.deadline();

        if check {
            self.server_ctx
                .num_early_returns
                .fetch_add(1, Ordering::Relaxed);
        }

        if PRIO_GLOBAL_TWO || PRIO_LOCAL_TWO {
            return check;
        } else {
            return false;
        }
        //}
        // }
        false
    }

    #[inline]
    fn issue_early_return<T>(&self) -> Result<Response<T>, Status> {
        Err(Status::new(Code::DeadlineExceeded, self.method.id()))
    }
}

impl RequestHandlerHooks for SimpleParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<SimpleServerContext>,
    ) -> Self {
        log::info!("parent_ctx, begin, method: {:?}", method.id());
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let ctx = Context::from_json(ctx_str);
        Self {
            method,
            ctx,
            server_ctx,
            //polled: AtomicUsize::new(0),
            request_start: Instant::now(),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        _child_send_ctx: &mut ChildContext,
    ) -> Option<Status> {
        if self.check_early_return() {
            return Some(Status::new(Code::DeadlineExceeded, self.method.id()));
        }

        log::info!("parent_ctx, before_child_rpc, method: {:?}", method.id());
        let graph = self
            .server_ctx
            .local_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .read()
            .unwrap();
        let deadline;
        let latest_exec_at;
        if FIFO || FIFO_TWO || PRIO_GLOBAL || PRIO_GLOBAL_TWO {
            deadline = self.ctx.deadline();
            latest_exec_at = self.ctx.latest_exec_at();
        } else if PRIO_LOCAL || PRIO_LOCAL_TWO {
            deadline = self.ctx.deadline() - graph.estimate_suffix_deadline(&method.id());
            latest_exec_at =
                self.ctx.deadline() - graph.estimate_suffix_latest_exec_at(&method.id());
        } else {
            panic!("Unimplemented policy");
        }
        let child_recv_ctx = Context::new(
            self.ctx.graph_id().clone(),
            self.ctx.test_id(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.request_class(),
            self.ctx.start_at(),
            deadline,
            latest_exec_at,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);
        None
    }

    fn after_child_rpc<T>(
        &self,
        child_rpc_method: GrpcMethod,
        _resp: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) -> Option<Status> {
        //if self.check_early_return() {
        //    return Some(Status::new(Code::DeadlineExceeded, self.method.id()));
        //}

        log::info!(
            "parent_ctx, after_child_rpc, method: {:?}",
            child_rpc_method.id()
        );
        let latency_us = child_ctx.track_latency.get_latency().unwrap().as_micros();
        let mut graph = self
            .server_ctx
            .local_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .write()
            .unwrap();
        graph.track_span(&child_rpc_method.id(), latency_us as u64);
        None
    }

    fn before_poll<Ret>(&self) -> Option<Result<Response<Ret>, Status>> {
        // log::info!("parent_ctx, before_poll, method: {:?}", self.method.id());
        // self.polled.fetch_add(1, Ordering::Relaxed);
        None
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Option<Result<Response<Ret>, Status>> {
        match poll {
            Poll::Pending => {
                if self.check_early_return() {
                    return Some(self.issue_early_return());
                }
            }
            Poll::Ready(res) => {}
        };
        None

        // if self.method.id() == "/frontend.Frontend/HandleSearch" {
        //     if let Poll::Ready(resp) = poll {
        //         let request_id = self.ctx.request_id();
        //         let graph_id = self.ctx.graph_id();
        //         let slo = 0; // [TODO]
        //         let latency = self.request_start.elapsed().as_micros();
        //         let error = resp.is_err();
        //         log::warn!(
        //             "{},{},{},{},{},{}",
        //             self.method.id(),
        //             request_id,
        //             graph_id,
        //             slo,
        //             latency,
        //             error
        //         );
        //     }
        // }
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
            method,
            track_latency: LatencyTracker::NotStarted,
        }
    }

    fn before_send<T>(&mut self, _req: &mut Request<T>) {
        log::info!("child_ctx, before_send, method: {:?}", self.method.id());
        self.track_latency.start();
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        self.track_latency.record_latency();
        log::info!(
            "child_ctx, after_recv, method: {:?}, elapsed: {} us",
            self.method.id(),
            self.track_latency.get_latency().unwrap().as_micros(),
        );
    }
}

impl SimpleServerContext {
    /// Construct a SimpleServerContext.
    pub fn new(service_name: &'static str) -> Self {
        // [NOTE] Hierarachy:
        // - Application
        //  - Service
        //   - Method
        //    - Span (Method / Compute)

        // let global_graph =
        //     mock_graph::charleston::get_global_graph_i2(service_name.to_string(), Some(100), 1_000);
        let global_graph = mock_graph::hotel::get_global_graph(service_name.to_string());

        // [CL] Ideally, trackers should be Hashmap<MethodId, LatencyTracker>.
        // But for now, two trackers are tracking the same method ID separately.
        // Namely, each local graph has its own tracker.

        let local_graphs = global_graph.local_graphs().clone();
        let local_graph_trackers = local_graphs
            .iter()
            .map(|(path, local_graph)| {
                let local_graph = LocalGraphTracker::from(local_graph.clone());
                (path.clone(), RwLock::new(local_graph))
            })
            .collect();

        log::info!(
            "SimpleServerContext, service: {:?}, local_graphs: {:?}",
            service_name,
            local_graphs
        );

        let num_early_returns = Arc::new(AtomicUsize::new(0));
        let errs = num_early_returns.clone();
        tokio::spawn(async move {
            let mut last = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let curr = errs.load(Ordering::Relaxed);
                log::warn!("Num errs: {} (diff {})", curr, curr - last);
                last = curr;
            }
        });

        Self {
            service_name,
            num_early_returns,
            local_graphs,
            local_graph_trackers,
        }
    }
}
