//! Masa hook traits and runtime support for tonic integration.
//!
//! Tonic owns the hook contracts and exact Tonic boundary types. Policy
//! implementations live outside Tonic and implement these traits.

use std::{sync::Arc, task::Poll};

use crate::body::BoxBody;
use crate::{http, GrpcMethod, Request, Response, Status};

// TODO: add notes on trait bounds
/// Trait for specifying the set of hooks to apply.
pub trait Hooks: Send + Sync + 'static {
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

    /// Invoked after the request handler (or the hook) has produced a response, but before it is serialized.
    ///
    /// This is useful for inspecting the response before it is serialized.
    ///
    /// `finalize_after_serialization` is invoked after this hook and after the response is serialized.
    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {}

    /// The last lifecycle hook to be invoked. Provides a mutable reference to the response about
    /// to be sent back to the client.
    fn finalize_after_serialization(&self, response: &mut http::Response<BoxBody>) {}
}

#[cfg(test)]
mod tests {
    use super::{ClientHooks, ParentHooks, ServerHooks};
    use crate::masa::{client, future::Abortable, noop, server};
    use crate::{http, GrpcMethod, Request, Response};
    use std::cell::Cell;
    use std::future::{poll_fn, Future};
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    #[test]
    fn noop_hooks_keep_default_lifecycle_empty() {
        let method = GrpcMethod::new("TestService", "TestMethod");
        let server_ctx = Arc::new(noop::ServerContext::new("TestService"));
        let req = http::Request::new(());
        let parent = noop::ParentContext::begin(method, &req, server_ctx);
        let mut child_req = Request::new(());
        let mut child = noop::ChildContext::new(method, &child_req);

        parent
            .before_child_rpc(method, &mut child_req, &mut child)
            .unwrap();

        let mut response = Ok(Response::new(()));
        parent
            .after_child_rpc(method, &mut response, child)
            .expect("noop after_child_rpc should not fail");
        parent
            .before_poll::<()>()
            .expect("noop before_poll should not fail");
    }

    #[test]
    fn parent_context_tls_round_trips_for_matching_hooks() {
        let method = GrpcMethod::new("TestService", "TestMethod");
        let server_ctx = Arc::new(noop::ServerContext::new("TestService"));
        let req = http::Request::new(());
        let parent = noop::ParentContext::begin(method, &req, server_ctx);

        server::set_parent_ctx::<noop::NoopHooks>(&parent);
        let observed = unsafe { client::get_parent_ctx::<noop::NoopHooks>() };
        assert!(std::ptr::eq(observed.unwrap(), &parent));

        server::reset_parent_ctx::<noop::NoopHooks>();
        assert!(unsafe { client::get_parent_ctx::<noop::NoopHooks>() }.is_none());
    }

    #[test]
    fn abortable_future_before_poll_can_return_without_polling_inner() {
        let polled = Cell::new(false);
        let inner = poll_fn(|_| {
            polled.set(true);
            Poll::Ready(1_u8)
        });
        let mut fut = Box::pin(
            inner
                .abortable()
                .before_poll(|| Some(2_u8))
                .after_poll(|_| Some(3_u8))
                .build(),
        );
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert_eq!(fut.as_mut().poll(&mut cx), Poll::Ready(2));
        assert!(!polled.get());
    }

    #[test]
    fn abortable_future_after_poll_can_override_ready_output() {
        let inner = poll_fn(|_| Poll::Ready(1_u8));
        let mut fut = Box::pin(
            inner
                .abortable()
                .before_poll(|| None::<u8>)
                .after_poll(|poll| match poll {
                    Poll::Ready(1) => Some(2_u8),
                    _ => None,
                })
                .build(),
        );
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert_eq!(fut.as_mut().poll(&mut cx), Poll::Ready(2));
    }
}
