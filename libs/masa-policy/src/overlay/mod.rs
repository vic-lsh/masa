// Overlay module — policy-specific hooks layered on top of the base hook state.
//
// Defines the `Overlay`, `OverlayServer`, and `OverlayChild` traits that all
// overlay implementations conform to. Implementations are selected at compile
// time via `#[cfg]` re-exports, so there is no dynamic dispatch cost.
//
// Currently two overlays exist:
// - `predictive` (feature `est`): latency estimation, deadline tightening,
//   predictive admission control.
// - `rajomon` (feature `ac_rajomon`): token-based admission control with
//   server-side price signals.
//
// These are mutually exclusive. When neither is enabled, the `noop` overlay
// compiles every method away to nothing.

use std::task::Poll;

use masa_core::{Context, ContextBuilder, PriorityHint};
use tonic_core::{CowGrpcMethod, Response, Status};

// ── Submodules ──────────────────────────────────────────────────────────

#[cfg(feature = "est")]
pub(crate) mod predictive;

#[cfg(feature = "ac_rajomon")]
pub mod rajomon;

#[cfg(not(any(feature = "est", feature = "ac_rajomon")))]
mod noop;

// ── Traits ──────────────────────────────────────────────────────────────

/// Server-level overlay state, shared across all requests.
pub(crate) trait OverlayServer: Send + Sync + std::fmt::Debug {
    fn new() -> Self;
}

/// Result of [`Overlay::before_child_rpc`] — the child's deadline, priority,
/// and context builder populated by the overlay.
#[allow(dead_code)]
pub(crate) struct ChildRpcContext {
    pub deadline: u64,
    pub prio_hint: PriorityHint,
    pub builder: ContextBuilder,
}

/// Per-request overlay state. Hooks into the request lifecycle at key points
/// to implement policy-specific logic (estimation, admission control, etc.).
pub(crate) trait Overlay: Send + Sync + std::fmt::Debug {
    type Server: OverlayServer;
    type Child: OverlayChild;

    /// Construct per-request overlay state.
    ///
    /// May inspect and mutate `ctx` (e.g., Rajomon deducts tokens here).
    fn new(method: &CowGrpcMethod, server: &Self::Server, ctx: &mut Context) -> Self;

    /// Called before each poll of the handler future.
    ///
    /// Returns `Err` to abort the request (e.g., Rajomon drop, predictive
    /// reprioritization).
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>>;

    /// Called before each outbound child RPC.
    ///
    /// The overlay may reject the child RPC (returning `Err`), tighten the
    /// deadline, adjust priority, or set tokens on the context builder.
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method: &CowGrpcMethod,
        child_ctx: &mut Self::Child,
        request: &mut tonic_core::Request<T>,
        builder: ContextBuilder,
        slo_abort_error: impl FnOnce() -> Status,
    ) -> Result<ChildRpcContext, Status>;

    /// Called after a child RPC response is received.
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: &Self::Child,
    ) -> Result<(), Status>;

    /// Called after each poll of the handler future.
    fn after_poll<Ret>(&self, poll: &Poll<Result<Response<Ret>, Status>>);

    /// Called before the response is serialized and sent.
    fn finalize<Ret>(&self, ctx: &Context, result: &mut Result<Response<Ret>, Status>);
}

/// Per-child-RPC overlay state.
pub(crate) trait OverlayChild: Send + Sync + Clone + std::fmt::Debug {
    fn new() -> Self;
}

// ── Compile-time selection ──────────────────────────────────────────────

#[cfg(all(feature = "est", feature = "ac_rajomon"))]
compile_error!(
    "Features `est` and `ac_rajomon` are mutually exclusive. Use one overlay at a time."
);

#[cfg(feature = "ac_rajomon")]
pub(crate) use rajomon::{
    RajomonOverlay as ActiveOverlay, RajomonOverlayChild as ActiveOverlayChild,
    RajomonOverlayServer as ActiveOverlayServer,
};

#[cfg(all(feature = "est", not(feature = "ac_rajomon")))]
pub(crate) use predictive::{
    PredictiveOverlay as ActiveOverlay, PredictiveOverlayChild as ActiveOverlayChild,
    PredictiveOverlayServer as ActiveOverlayServer,
};

#[cfg(not(any(feature = "est", feature = "ac_rajomon")))]
pub(crate) use noop::{
    NoopOverlay as ActiveOverlay, NoopOverlayChild as ActiveOverlayChild,
    NoopOverlayServer as ActiveOverlayServer,
};
