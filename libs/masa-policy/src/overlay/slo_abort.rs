// SLO abort overlay — rejects requests that have exceeded their end-to-end
// SLO deadline. Wraps the existing `SloAbortHandler`.

use std::task::Poll;

use masa_core::Context;
use tonic_core::{CowGrpcMethod, Response, Status};

use super::{ChildRpcContext, Overlay, OverlayChild, OverlayServer};
use crate::slo_abort::SloAbortHandler;

// ── Server ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct SloAbortOverlayServer;

impl OverlayServer for SloAbortOverlayServer {
    fn new() -> Self {
        Self
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct SloAbortOverlay {
    handler: SloAbortHandler,
}

impl Overlay for SloAbortOverlay {
    type Server = SloAbortOverlayServer;
    type Child = SloAbortOverlayChild;

    fn new(method: &CowGrpcMethod, _server: &SloAbortOverlayServer, _ctx: &mut Context) -> Self {
        Self {
            handler: SloAbortHandler::new(method.clone()),
        }
    }

    #[inline]
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        if self.handler.check(ctx) {
            return Err(Err(self.handler.issue_error()));
        }
        Ok(())
    }

    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        _child_ctx: &mut SloAbortOverlayChild,
        _request: &mut tonic_core::Request<T>,
        _child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        if self.handler.check(ctx) {
            return Err(self.handler.issue_error());
        }
        Ok(())
    }

    #[inline]
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        child_method: &CowGrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        _child_ctx: &SloAbortOverlayChild,
    ) -> Result<(), Status> {
        self.handler.set_last_child(child_method.clone());
        Ok(())
    }

    #[inline]
    fn after_poll<Ret>(
        &self,
        ctx: &Context,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.handler.check(ctx) {
                return Err(Err(self.handler.issue_error()));
            }
        }
        Ok(())
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct SloAbortOverlayChild;

impl OverlayChild for SloAbortOverlayChild {
    fn new() -> Self {
        Self
    }
}
