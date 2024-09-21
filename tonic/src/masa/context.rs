use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
    time::Instant,
};

use tonic_masa::Context;

use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};

/// A simple implementation of `ClientStubHooks`.
#[derive(Debug)]
pub struct SimpleChildContext {
    method: GrpcMethod,
    start: Option<Instant>,
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

/// A simple implementation of `RequestHandlerHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SimpleParentContext {
    method: GrpcMethod,
    req_ctx: Context,
    server_ctx: Arc<ServerContext>,

    polled: AtomicUsize,
    request_start: Instant,
}

/// Context struct for an RPC server, instantiated during server startup.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    // local_graph: Option<LocalGraph>,
}

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
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let req_ctx = Context::from_json(ctx_str);
        Self {
            method,
            req_ctx,
            server_ctx,
            polled: AtomicUsize::new(0),
            request_start: Instant::now(),
        }
    }

    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _req: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) {
        println!("before_child_rpc, {:?}", method);
    }

    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _resp: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) {
        println!("after_child_rpc, {:?}", method);
    }

    fn before_poll(&self) {
        self.polled.fetch_add(1, Ordering::Relaxed);
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        println!(
            "method {:?} polled {} times, duration {} ms",
            self.method,
            self.polled.load(Ordering::Relaxed),
            self.request_start.elapsed().as_millis()
        );
    }
}

impl ClientStubHooks for SimpleChildContext {
    fn new<T>(method: GrpcMethod, _req: &Request<T>) -> Self {
        Self {
            method,
            start: None,
        }
    }

    fn before_send<T>(&mut self, _req: &mut Request<T>) {
        println!("child_ctx {:?} before send", self.method);
        self.start = Some(Instant::now());
    }

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
        let lat_ms = self
            .start
            .take()
            .expect("rpc must have started")
            .elapsed()
            .as_millis();
        println!(
            "child_ctx {:?} after recv, latency {} ms",
            self.method, lat_ms
        );
    }
}

impl ServerContext {
    /// Construct a ServerContext.
    pub fn new(service_name: &'static str) -> Self {
        println!("ServerContext: constructed for service {}", service_name);
        Self { service_name }
    }
}
