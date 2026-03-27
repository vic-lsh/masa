// Overlay module — composable hooks layered on the base request lifecycle.
//
// Defines the `Overlay`, `OverlayServer`, and `OverlayChild` traits and the
// `overlay_stack!` macro that composes multiple overlays into the generated
// `ParentContext`, `ServerContext`, `ChildContext`, and hook impls.
//
// Three overlay categories:
// - **Guard**: `SloAbortOverlay` — rejects past-deadline requests.
// - **Policy** (mutually exclusive, compile-time selected):
//   - `predictive` (feature `estimator`): latency estimation, deadline tightening,
//     predictive admission control.
//   - `rajomon` (feature `ac_rajomon`): token-based admission control with
//     server-side price signals.
//   - `noop`: when neither is enabled, compiles away to nothing.
// - **Observer**: `QueueLatOverlay` — tracks queue latencies.
//
// All dispatch is monomorphic — zero runtime cost.

use std::task::Poll;

use masa_core::{Context, PriorityHint};
use tonic_core::{CowGrpcMethod, Response, Status};

// ── Submodules ──────────────────────────────────────────────────────────

#[cfg(feature = "estimator")]
pub(crate) mod predictive;

#[cfg(feature = "ac_rajomon")]
pub mod rajomon;

#[cfg(not(any(feature = "estimator", feature = "ac_rajomon")))]
mod noop;

mod queue_lat;
mod slo_abort;

// ── Traits ──────────────────────────────────────────────────────────────

/// Server-level overlay state, shared across all requests.
pub(crate) trait OverlayServer: Send + Sync + std::fmt::Debug {
    fn new() -> Self;
}

/// Mutable state populated by overlays in [`Overlay::before_child_rpc`].
///
/// Initialized from the parent context via [`ChildRpcContext::from_parent`].
/// Each overlay in the stack may mutate fields (e.g., tighten deadline, set
/// tokens). After all overlays have run, hooks builds the child `Context`
/// from these fields.
pub(crate) struct ChildRpcContext {
    pub deadline: u64,
    pub prio_hint: PriorityHint,
    pub hop_count: u8,
    pub tokens: u64,
}

impl ChildRpcContext {
    pub fn from_parent(ctx: &Context) -> Self {
        // hop_count is only incremented when estimation-based admission control
        // is active — it uses hop_count to distinguish ingress from internal hops.
        let hop_count = if cfg!(feature = "estimator") {
            ctx.hop_count().saturating_add(1)
        } else {
            ctx.hop_count()
        };
        Self {
            deadline: ctx.deadline(),
            prio_hint: ctx.prio_hint(),
            hop_count,
            tokens: ctx.tokens(),
        }
    }
}

/// Per-request overlay state. Hooks into the request lifecycle at key points
/// to implement policy-specific logic (estimation, admission control, etc.).
///
/// All methods have default no-op implementations so that overlays only need
/// to override the hooks they care about.
pub(crate) trait Overlay: Send + Sync + std::fmt::Debug {
    type Server: OverlayServer;
    type Child: OverlayChild;

    /// Construct per-request overlay state.
    ///
    /// May inspect and mutate `ctx` (e.g., Rajomon deducts tokens here).
    fn new(method: &CowGrpcMethod, server: &Self::Server, ctx: &mut Context) -> Self;

    /// Called before each poll of the handler future.
    ///
    /// Returns `Err` to abort the request.
    fn before_poll<Ret>(&self, _ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Called before each outbound child RPC.
    ///
    /// The overlay may reject the child RPC (returning `Err`) or mutate
    /// `child_rpc` to tighten the deadline, adjust priority, or set tokens.
    fn before_child_rpc<T>(
        &self,
        _ctx: &Context,
        _child_method: &CowGrpcMethod,
        _child_ctx: &mut Self::Child,
        _request: &mut tonic_core::Request<T>,
        _child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Called after a child RPC response is received.
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        _child_method: &CowGrpcMethod,
        _response: &mut Result<Response<T>, Status>,
        _child_ctx: &Self::Child,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Called after each poll of the handler future.
    ///
    /// Returns `Err` to abort the request (e.g., SLO abort on `Pending`).
    fn after_poll<Ret>(
        &self,
        _ctx: &Context,
        _poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Called before the response is serialized and sent.
    ///
    /// Overlays should mutate `ctx` directly (e.g., set `response_meta` or
    /// `queue_latencies`). The caller serializes the context once after all
    /// overlays have run.
    fn finalize<Ret>(&self, _ctx: &mut Context, _result: &mut Result<Response<Ret>, Status>) {}
}

/// Per-child-RPC overlay state.
pub(crate) trait OverlayChild: Send + Sync + Clone + std::fmt::Debug {
    fn new() -> Self;
}

// ── Re-exports ──────────────────────────────────────────────────────────

pub(crate) use queue_lat::QueueLatOverlay;
pub(crate) use slo_abort::SloAbortOverlay;

// ── Compile-time policy overlay selection ────────────────────────────────

#[cfg(all(feature = "estimator", feature = "ac_rajomon"))]
compile_error!(
    "Features `estimator` and `ac_rajomon` are mutually exclusive. Use one overlay at a time."
);

#[cfg(feature = "ac_rajomon")]
pub(crate) use rajomon::RajomonOverlay as AdmissionControlOverlay;

#[cfg(all(feature = "estimator", not(feature = "ac_rajomon")))]
pub(crate) use predictive::PredictiveOverlay as AdmissionControlOverlay;

#[cfg(not(any(feature = "estimator", feature = "ac_rajomon")))]
pub(crate) use noop::NoopOverlay as AdmissionControlOverlay;
