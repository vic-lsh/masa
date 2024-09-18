use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::Poll,
    time::Instant,
};

use tonic_masa::Context;

use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};

pub struct RequestTxContext {}

/// Context struct instantiated once per RPC, when the server invokes a request handler.
///
/// Must implement `RequestHandlerHooks`.
pub type RequestRxContext = SimpleReqRxCtx;

/// Type of metadata required for async-tasks used in tonic.
pub type AsyncTaskMetadata = Arc<Mutex<Option<RequestRxContext>>>;

/// A simple implementation of `RequestHandlerHooks`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SimpleReqRxCtx {
    method_name: &'static str,
    req_ctx: Context,
    server_ctx: Arc<ServerContext>,

    polled: AtomicUsize,
    request_start: Instant,
}

// [NOTE] Tonic-generated server requires Debug.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    // local_graph: Option<LocalGraph>,
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
    fn begin<B>(
        method: &'static str,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self;

    /// Invoked before the request handler makes an RPC.
    fn before_child_rpc<T>(&self, method: GrpcMethod, req: &mut Request<T>) {}

    /// Invoked after the request handler receives a response from an RPC it made earlier.
    fn after_child_rpc<T>(&self, method: GrpcMethod, resp: &mut Result<Response<T>, Status>) {}

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

impl RequestHandlerHooks for SimpleReqRxCtx {
    fn begin<B>(
        method: &'static str,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let req_ctx = Context::from_json(ctx_str);
        Self {
            method_name: method,
            req_ctx,
            server_ctx,
            polled: AtomicUsize::new(0),
            request_start: Instant::now(),
        }
    }

    fn before_child_rpc<T>(&self, method: GrpcMethod, req: &mut Request<T>) {
        println!("before_child_rpc, {:?}", method);
    }

    fn after_child_rpc<T>(&self, method: GrpcMethod, resp: &mut Result<Response<T>, Status>) {
        println!("after_child_rpc, {:?}", method);
    }

    fn before_poll(&self) {
        self.polled.fetch_add(1, Ordering::Relaxed);
    }

    fn finalize(&self, response: &mut http::Response<BoxBody>) {
        println!(
            "method {} polled {} times, duration {} ms",
            self.method_name,
            self.polled.load(Ordering::Relaxed),
            self.request_start.elapsed().as_millis()
        );
    }
}

impl ServerContext {
    pub fn new(service_name: &'static str) -> Self {
        println!("ServerContext: constructed for service {}", service_name);
        Self { service_name }
    }
}
