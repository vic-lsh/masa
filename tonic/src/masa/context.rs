use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    task::Poll,
    time::Instant,
};

use tonic_masa::{
    Context, LocalGraph, LocalGraphTracker, MethodId, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_LOCAL,
};

use crate::{body::BoxBody, masa::mock_graph, GrpcMethod, Request, Response, Status};

/// A simple implementation of `RequestHandlerHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SimpleParentContext {
    method: GrpcMethod,
    ctx: Context,
    server_ctx: Arc<ServerContext>,

    polled: AtomicUsize,
    request_start: Instant,
}

/// A simple implementation of `ClientStubHooks`.
#[derive(Debug)]
pub struct SimpleChildContext {
    method: GrpcMethod,
    start: Option<Instant>,
    latency_us: Option<u64>,
}

/// Context struct for an RPC server, instantiated during server startup.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    local_graphs: HashMap<MethodId, LocalGraph>,
    local_graph_trackers: HashMap<MethodId, RwLock<LocalGraphTracker>>,
}

/// Context struct instantiated once per RPC, when the server invokes a request handler.
///
/// Must implement `RequestHandlerHooks`.
pub type ParentContext = SimpleParentContext;

/// Context struct instantiated on RPC transmission.
///
/// Must implement `ClientStubHooks`.
pub type ChildContext = SimpleChildContext;

/// Type of metadata required for async-tasks used in tonic.
pub type AsyncTaskMetadata = Option<Arc<ParentContext>>;

/// Lifecycle hooks when a client stub executes a request.
///
/// Structs that implement this trait (aliased to `ChildContext`) will be
/// constructed each time a client stub sends an RPC, and destructeed when the
/// RPC completes.
///
/// All hooks have a default, empty implementation. The only exception is `new`,
/// which is the hook constructor and must be implemented.
///
/// This struct does not need to be thread-safe -- it will not be accessed concurrently.
#[allow(unused_variables)]
pub trait ClientStubHooks {
    /// Construct a new ClientStubHook.
    fn new<T>(method: GrpcMethod, req: &Request<T>) -> Self;

    /// Lifecycle hook invoked just before a client stub sends an RPC.
    ///
    /// This is the last lifecycle hook to be called before the request is sent.
    /// For example, it is called _after_ hooks like `RequestHandlerHooks::before_child_rpc`.
    fn before_send<T>(&mut self, req: &mut Request<T>) {}

    /// Lifecycle hook invoked right after a client stub received a response
    /// for this RPC.
    ///
    /// This is the first lifecycle hook to be called after receiving the response.
    /// For example, it is called _before_ hooks like `RequestHandlerHooks::after_child_rpc`.
    fn after_recv<T>(&mut self, response: &mut Result<Response<T>, Status>) {}
}

/// Lifecycle hooks when the server executes a request.
///
/// Structs that implement this trait will be constructed each time a server
/// starts handling a request, and destructed when the request is complete.
///
/// All hook points have a default, empty implementation (except for `begin`,
/// which must be implemented and acts as a constructor). Implementer can choose
/// to only implement hooks they're interested in.
///
/// The implementation must be multithread-safe (i.e. `Sync`). One reasons is
/// that child RPCs can run in parallel, and they may invoke hook points
/// defined below from different threads.
#[allow(unused_variables)]
pub trait RequestHandlerHooks: Sync {
    /// The first lifecycle, marking the start of a request execution.
    ///
    /// This is also the constructor for the hook point struct implementation.
    fn begin<B>(method: GrpcMethod, req: &http::Request<B>, server_ctx: Arc<ServerContext>)
        -> Self;

    /// Invoked before the request handler makes an RPC.
    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        req: &mut Request<T>,
        child_ctx: &mut ChildContext,
    ) {
    }

    /// Invoked after the request handler receives a response from an RPC it made earlier.
    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        resp: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) {
    }

    /// Invoked each time before the request handler is polled.
    ///
    /// This indicates that the request handler can make progress.
    fn before_poll(&self) {}

    /// Invoked each time after the request handler is polled.
    ///
    /// The poll result shows whether the request is blocked or finalized.
    fn after_poll<T>(&self, poll: &Poll<T>) {}

    /// The last lifecycle hook to be invoked. Provides a mutable reference to the response about
    /// to be sent back to the client.
    fn finalize(&self, response: &mut http::Response<BoxBody>) {}
}

impl RequestHandlerHooks for SimpleParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        log::info!("parent_ctx, begin, method: {:?}", method.id());
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let ctx = Context::from_json(ctx_str);
        Self {
            method,
            ctx,
            server_ctx,
            polled: AtomicUsize::new(0),
            request_start: Instant::now(),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        _child_send_ctx: &mut ChildContext,
    ) {
        log::info!("parent_ctx, before_child_rpc, method: {:?}", method.id());
        let deadline;
        let latest_exec_at;
        if PRIO_GLOBAL || FIFO_TWO || FIFO {
            deadline = self.ctx.deadline();
            latest_exec_at = self.ctx.latest_exec_at();
        } else {
            panic!("Unimplemented policy");
        }
        let child_recv_ctx = Context::new(
            self.ctx.graph_id().clone(),
            self.ctx.request_id(),
            deadline,
            latest_exec_at,
            self.ctx.request_class(),
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);
    }

    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _resp: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext,
    ) {
        log::info!("parent_ctx, after_child_rpc, method: {:?}", method.id());
        let latency_us = child_ctx.latency_us.unwrap();
        self.server_ctx
            .local_graph_trackers
            .get(&self.method.id())
            .unwrap()
            .write()
            .unwrap()
            .track_span(&method.id(), latency_us);
    }

    fn before_poll(&self) {
        log::info!("parent_ctx, before_poll, method: {:?}", self.method.id());
        self.polled.fetch_add(1, Ordering::Relaxed);
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        log::info!(
            "parent_ctx, finalize, method: {:?}, polled: {} times, elapsed: {} us",
            self.method.id(),
            self.polled.load(Ordering::Relaxed),
            self.request_start.elapsed().as_micros()
        );
    }
}

impl ClientStubHooks for SimpleChildContext {
    fn new<T>(method: GrpcMethod, _req: &Request<T>) -> Self {
        Self {
            method,
            start: None,
            latency_us: None,
        }
    }

    fn before_send<T>(&mut self, _req: &mut Request<T>) {
        log::info!("child_ctx, before_send, method: {:?}", self.method.id());
        self.start = Some(Instant::now());
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        let elapsed_us = self
            .start
            .take()
            .expect("Child RPC must have started")
            .elapsed()
            .as_micros() as u64;
        log::info!(
            "child_ctx, after_recv, method: {:?}, elapsed: {} us",
            self.method.id(),
            elapsed_us
        );
        self.latency_us = Some(elapsed_us);
    }
}

impl ServerContext {
    /// Construct a ServerContext.
    pub fn new(service_name: &'static str) -> Self {
        // Hierarachy:
        // - Application
        //  - Service
        //   - Method
        //    - Span (Method / Compute)

        let global_graph =
            mock_graph::get_global_graph_i2(service_name.to_string(), Some(100), 1_000);

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
            "ServerContext, service: {}, local_graphs: {:?}",
            service_name,
            local_graphs
        );

        Self {
            service_name,
            local_graphs,
            local_graph_trackers,
        }
    }
}
