use std::{sync::Arc, task::Poll};

use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};

mod simple;

/// Context struct for an RPC server, instantiated during server startup.
pub type ServerContext = simple::SimpleServerContext;

/// Context struct instantiated once per RPC, when the server invokes a request handler.
///
/// Must implement `RequestHandlerHooks`.
pub type ParentContext = simple::SimpleParentContext;

/// Context struct instantiated on RPC transmission.
///
/// Must implement `ClientStubHooks`.
pub type ChildContext = simple::SimpleChildContext;

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
/// The implementation must be multithread-safe (i.e. `Send+Sync`). One reason
/// is that child RPCs can run in parallel, and they may invoke hook points
/// defined below from different threads.
#[allow(unused_variables)]
pub trait RequestHandlerHooks<Child>: Send + Sync
where
    Child: ClientStubHooks,
{
    /// The first lifecycle, marking the start of a request execution.
    ///
    /// This is also the constructor for the hook point struct implementation.
    fn begin<B>(method: GrpcMethod, req: &http::Request<B>, server_ctx: Arc<ServerContext>)
        -> Self;

    /// Invoked before the request handler makes an RPC.
    fn before_child_rpc<T>(&self, method: GrpcMethod, req: &mut Request<T>, child_ctx: &mut Child) {
    }

    /// Invoked after the request handler receives a response from an RPC it made earlier.
    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        resp: &mut Result<Response<T>, Status>,
        child_ctx: Child,
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
