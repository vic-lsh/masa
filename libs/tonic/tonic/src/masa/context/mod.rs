use std::{sync::Arc, task::Poll};

use crate::{body::BoxBody, GrpcMethod, Request, Response, Status};

mod global;
mod local;
mod noop;
mod perfect_lsf;
mod queue_global;
mod queue_tracing;
mod simple;
mod tracing;

pub mod runtime;
mod tls;
use masa::Context;
pub use tls::{client, server};

#[cfg(not(feature = "masa"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = noop::NoopPrioritySelector;
// pub type DefaultPrioritySelector = queue_tracing::QueueTracing;

#[cfg(any(feature = "fifo"))]
#[allow(missing_docs)]
// TODO: revert back to noop for Fifo. Add another feature flag for tracing.
// pub type DefaultPrioritySelector = noop::NoopPrioritySelector;
// pub type DefaultPrioritySelector = tracing::Tracing;
pub type DefaultPrioritySelector = noop::NoopPrioritySelector;

#[cfg(any(feature = "fifo_span_tracing"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = tracing::Tracing;

#[cfg(any(feature = "fifo_queue_tracing"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = queue_tracing::QueueTracing;

#[cfg(any(feature = "prio_local"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = simple::SimplePrioritySelector;

#[cfg(any(feature = "prio_global"))]
#[allow(missing_docs)]
// pub type DefaultPrioritySelector = queue_global::QueueGlobal;
pub type DefaultPrioritySelector = global::Global;

#[cfg(any(feature = "prio_global_queue_tracing"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = queue_global::QueueGlobal;

#[cfg(any(feature = "prio_local_direct"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = local::LocalDeadlineDirect;

#[cfg(any(feature = "prio_local_indirect"))]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = local::LocalDeadlineIndirect;

#[cfg(feature = "perfect_lsf")]
#[allow(missing_docs)]
pub type DefaultPrioritySelector = perfect_lsf::PerfectLSF;

// TODO: rename this to be more general
// TODO: add notes on trait bounds
/// Trait for specifying the set of hooks to apply in a Masa build.
pub trait PrioritySelector: Send + Sync + 'static {
    /// The server-level state and hook implementations.
    type ServerContext: ServerHooks;
    /// The state and hook implementations maintained per child RPC.
    type ChildContext: ClientHooks;
    /// The state and hook implementations maintained per in the server-side request handlers.
    type ParentContext: ParentHooks<Self::ChildContext, Self::ServerContext>;
}

/// Lifecycle hooks of a Masa server.
#[allow(unused_variables)]
pub trait ServerHooks: Send + Sync + 'static {
    /// Creates the service-level context.
    // [TODO:Vic] mark this function as async to support fetching resources
    // asynchronously. this may require async support in the tonic service constructor.
    fn new(service_name: &'static str) -> Self;
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
pub trait ClientHooks {
    /// Construct a new ClientStubHook.
    fn new<T>(method: GrpcMethod, request: &Request<T>) -> Self;

    /// Lifecycle hook invoked just before a client stub sends an RPC.
    ///
    /// This is the last lifecycle hook to be called before the request is sent.
    /// For example, it is called _after_ hooks like `ParentHooks::before_child_rpc`.
    fn before_send<T>(&mut self, request: &mut Request<T>) {}

    /// Lifecycle hook invoked right after a client stub received a response
    /// for this RPC.
    ///
    /// This is the first lifecycle hook to be called after receiving the response.
    /// For example, it is called _before_ hooks like `ParentHooks::after_child_rpc`.
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
pub trait ParentHooks<Child, Server>: Send + Sync
where
    Child: ClientHooks,
    Server: ServerHooks,
{
    /// The first lifecycle, marking the start of a request execution.
    ///
    /// This is also the constructor for the hook point struct implementation.
    fn begin<B>(method: GrpcMethod, req: &http::Request<B>, server_ctx: Arc<Server>) -> Self;

    /// Invoked before the request handler makes an RPC.
    #[must_use]
    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut Child,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Invoked after the request handler receives a response from an RPC it made earlier.
    #[must_use]
    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: Child,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Invoked each time before the request handler future is polled.
    ///
    /// Being invoked indicates that the request handler can make progress.
    ///
    /// To return early without continuing request processing, return an error
    /// with the response to send back to the client.
    #[must_use]
    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Invoked each time after the request handler future is polled.
    ///
    /// The poll result shows whether the request is blocked or finalized.
    ///
    /// To return early without continuing request processing, return an error
    /// with the response to send back to the client.
    #[must_use]
    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// The last lifecycle hook to be invoked. Provides a mutable reference to the response about
    /// to be sent back to the client.
    fn finalize(&self, response: &mut http::Response<BoxBody>) {}
}

#[allow(dead_code)]
fn read_context<B>(req: &http::Request<B>) -> Context {
    let ctx_str = req.headers()["ctx"].to_str().unwrap();
    Context::from_json(ctx_str)
}
