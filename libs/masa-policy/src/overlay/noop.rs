// No-op overlay — used when neither `est` nor `ac_rajomon` is enabled.
//
// All methods are no-ops that compile away entirely.

use std::task::Poll;

use masa_core::{Context, ContextBuilder};
use tonic_core::{CowGrpcMethod, Response, Status};

use super::{ChildRpcContext, Overlay, OverlayChild, OverlayServer};

#[derive(Debug)]
pub(crate) struct NoopOverlayServer;

impl OverlayServer for NoopOverlayServer {
    fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct NoopOverlay;

impl Overlay for NoopOverlay {
    type Server = NoopOverlayServer;
    type Child = NoopOverlayChild;

    fn new(_method: &CowGrpcMethod, _server: &NoopOverlayServer, _ctx: &mut Context) -> Self {
        Self
    }

    #[inline]
    fn before_poll<Ret>(&self, _ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        _child_ctx: &mut NoopOverlayChild,
        _request: &mut tonic_core::Request<T>,
        builder: ContextBuilder,
        _slo_abort_error: impl FnOnce() -> Status,
    ) -> Result<ChildRpcContext, Status> {
        Ok(ChildRpcContext {
            deadline: ctx.deadline(),
            prio_hint: ctx.prio_hint(),
            builder: builder.deadline(ctx.deadline()).prio_hint(ctx.prio_hint()),
        })
    }

    #[inline]
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        _child_method: &CowGrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        _child_ctx: &NoopOverlayChild,
    ) -> Result<(), Status> {
        Ok(())
    }

    #[inline]
    fn after_poll<Ret>(&self, _poll: &Poll<Result<Response<Ret>, Status>>) {}

    #[inline]
    fn finalize<Ret>(&self, _ctx: &Context, _result: &mut Result<Response<Ret>, Status>) {}
}

#[derive(Debug, Clone)]
pub(crate) struct NoopOverlayChild;

impl OverlayChild for NoopOverlayChild {
    fn new() -> Self {
        Self
    }
}
