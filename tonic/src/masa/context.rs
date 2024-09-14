use std::sync::Arc;

use tonic_masa::Context;

use crate::{Request, Response, Status};

/// Description of a RPC about to be transmitted.
#[derive(Debug)]
pub struct RpcInfo {
    // [TODO] replace service_name + method_name with `GrpcMethod`.
    // we can't do this right now, because GrpcMethod is defined within `tonic`,
    // and `tonic-masa` depending on `tonic` would create a dependency cycle.
    pub service_name: &'static str,
    pub method_name: &'static str,
}

pub struct RequestTxContext {}

#[derive(Debug)]
#[allow(dead_code)]
pub struct RequestRxContext {
    req_ctx: Context,
    server_ctx: Arc<ServerContext>,
}

// [NOTE] Tonic-generated server requires Debug.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ServerContext {
    service_name: &'static str,
    // local_graph: Option<LocalGraph>,
}

impl RequestRxContext {
    pub fn new<B>(req: &http::Request<B>, server_ctx: Arc<ServerContext>) -> Self {
        let ctx_str = req.headers()["ctx"].to_str().unwrap();
        let req_ctx = Context::from_json(ctx_str);
        Self {
            req_ctx,
            server_ctx,
        }
    }
}

// Request lifecycle hooks.
// [TODO] extract this into a trait.
impl RequestRxContext {
    pub fn before_child_rpc<T>(&self, rpc: RpcInfo, req: &mut Request<T>) {
        println!("before_child_rpc, {:?}", rpc);
    }

    pub fn after_child_rpc<T>(&self, rpc: RpcInfo, resp: &mut Result<Response<T>, Status>) {
        println!("after_child_rpc, {:?}", rpc);
    }
}

impl ServerContext {
    pub fn new(service_name: &'static str) -> Self {
        println!("ServerContext: constructed for service {}", service_name);
        Self { service_name }
    }
}
