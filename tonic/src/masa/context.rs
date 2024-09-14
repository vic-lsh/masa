use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
};

use tonic_masa::Context;

use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};

pub struct RequestTxContext {}

#[derive(Debug)]
#[allow(dead_code)]
pub struct RequestRxContext {
    method_name: &'static str,
    req_ctx: Context,
    server_ctx: Arc<ServerContext>,
    polled: AtomicUsize,
}

// [NOTE] Tonic-generated server requires Debug.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    // local_graph: Option<LocalGraph>,
}

impl RequestRxContext {
    pub fn new<B>(
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
        }
    }
}

// Request lifecycle hooks.
// [TODO] extract this into a trait.
impl RequestRxContext {
    pub fn before_child_rpc<T>(&self, method: GrpcMethod, req: &mut Request<T>) {
        println!("before_child_rpc, {:?}", method);
    }

    pub fn after_child_rpc<T>(&self, method: GrpcMethod, resp: &mut Result<Response<T>, Status>) {
        println!("after_child_rpc, {:?}", method);
    }

    /// Invoked each time before the request handler is polled.
    ///
    /// This indicates that the request handler can make progress.
    pub fn before_poll(&self) {
        self.polled.fetch_add(1, Ordering::Relaxed);
    }

    /// Invoked each time after the request handler is polled.
    ///
    /// The poll result shows whether the request is blocked or finalized.
    pub fn after_poll<T>(&self, poll: &Poll<T>) {}

    /// The last lifecycle hook to be invoked. Provides a mutable reference to the response about
    /// to be sent back to the client.
    pub fn finalize(&self, response: &mut http::Response<BoxBody>) {
        println!(
            "method {} polled {} times",
            self.method_name,
            self.polled.load(Ordering::Relaxed)
        );
    }
}

impl ServerContext {
    pub fn new(service_name: &'static str) -> Self {
        println!("ServerContext: constructed for service {}", service_name);
        Self { service_name }
    }
}
