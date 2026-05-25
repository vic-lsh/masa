// Layer module — composable hooks layered on the base request lifecycle.
//
// Defines the `Layer`, `LayerServer`, and `LayerChild` traits used by the
// explicit layer containers in `hooks.rs`.
//
// Four layer categories:
// - **Guard**: `E2eDeadlineGuardLayer` — rejects past-deadline requests.
// - **Estimation** (feature `estimator`): `EstimationLayer` — latency tracking,
//   deadline tightening, reprioritization, feasibility checks.
// - **Oracle** (feature `sched_oracle`): `OracleLayer` — perfect-information
//   child deadline and priority assignment for synthetic experiments.
// - **Admission** (mutually exclusive, compile-time selected):
//   - `predictive` (feature `ac_pred`): goodput-tracking token-bucket AC.
//   - `rajomon` (feature `ac_rajomon`): token-based AC with price signals.
//   - `noop`: when neither AC is enabled, compiles away to nothing.
// - **Observer**: `QueueLatencyLayer` — tracks queue latencies.
//
// All dispatch is monomorphic — zero runtime cost.

use std::task::Poll;

use masa_core::{Context, PriorityHint};
use tonic::{CowGrpcMethod, Response, Status};

// ── Submodules ──────────────────────────────────────────────────────────

pub(crate) mod admission;

#[cfg(feature = "estimator")]
pub(crate) mod est;

mod e2e_deadline_guard;
mod oracle;
mod queue_latency;

// ── Traits ──────────────────────────────────────────────────────────────

/// Server-level layer state, shared across all requests.
pub(crate) trait LayerServer: Send + Sync + std::fmt::Debug {}

/// Mutable state populated by layers in [`Layer::before_child_rpc`].
///
/// Initialized from the parent context via [`ChildRpcContext::from_parent`].
/// Each layer in the stack may mutate fields (e.g., tighten deadline, set
/// tokens). After all layers have run, hooks builds the child `Context`
/// from these fields.
pub(crate) struct ChildRpcContext {
    pub deadline: u64,
    pub prio_hint: PriorityHint,
    #[cfg(feature = "estimator")]
    pub hop_count: u8,
    #[cfg(feature = "ac_rajomon")]
    pub tokens: u64,
}

impl ChildRpcContext {
    pub fn from_parent(ctx: &Context) -> Self {
        // hop_count is only incremented when estimation is active — it uses
        // hop_count to distinguish ingress from internal hops.
        Self {
            deadline: ctx.deadline(),
            prio_hint: ctx.prio_hint(),
            #[cfg(feature = "estimator")]
            hop_count: ctx.hop_count().saturating_add(1),
            #[cfg(feature = "ac_rajomon")]
            tokens: ctx.tokens(),
        }
    }
}

/// Per-request layer state. Hooks into the request lifecycle at key points
/// to implement policy-specific logic (estimation, admission control, etc.).
///
/// All methods have default no-op implementations so that layers only need
/// to override the hooks they care about.
pub(crate) trait Layer: Send + Sync + std::fmt::Debug {
    type Server: LayerServer;
    type Child: LayerChild;

    /// Construct per-request layer state.
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
    /// The layer may reject the child RPC (returning `Err`) or mutate
    /// `child_rpc` to tighten the deadline, adjust priority, or set tokens.
    fn before_child_rpc<T>(
        &self,
        _ctx: &Context,
        _child_method: &CowGrpcMethod,
        _child_ctx: &mut Self::Child,
        _request: &mut tonic::Request<T>,
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
    /// Returns `Err` to abort the request (e.g., deadline guard on `Pending`).
    fn after_poll<Ret>(
        &self,
        _ctx: &Context,
        _poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Called before the response is serialized and sent.
    ///
    /// Layers should mutate `ctx` directly (e.g., set `response_meta` or
    /// `queue_latencies`). The caller serializes the context once after all
    /// layers have run.
    fn finalize<Ret>(&self, _ctx: &mut Context, _result: &mut Result<Response<Ret>, Status>) {}
}

/// Per-child-RPC layer state.
pub(crate) trait LayerChild: Send + Sync + Clone + std::fmt::Debug {
    fn new() -> Self;
}

// ── Estimation layer type alias ────────────────────────────────────────

#[cfg(feature = "estimator")]
pub(crate) use est::EstimationLayer;

#[cfg(not(feature = "estimator"))]
pub(crate) use self::est_noop::NoopEstLayer as EstimationLayer;

#[cfg(not(feature = "estimator"))]
mod est_noop {
    use masa_core::Context;
    use tonic::CowGrpcMethod;

    use super::{Layer, LayerChild, LayerServer};

    #[derive(Debug)]
    pub(crate) struct NoopEstServer;

    impl NoopEstServer {
        pub(crate) fn new(_service_name: &'static str) -> Self {
            Self
        }
    }

    impl LayerServer for NoopEstServer {}

    #[derive(Debug)]
    pub(crate) struct NoopEstLayer;

    impl Layer for NoopEstLayer {
        type Server = NoopEstServer;
        type Child = NoopEstChild;

        fn new(_method: &CowGrpcMethod, _server: &NoopEstServer, _ctx: &mut Context) -> Self {
            Self
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct NoopEstChild;

    impl LayerChild for NoopEstChild {
        fn new() -> Self {
            Self
        }
    }
}

// ── Re-exports ──────────────────────────────────────────────────────────

#[cfg(feature = "ac_pred")]
pub(crate) use admission::AdmissionDeps;
pub(crate) use admission::{AdmissionLayer, AdmissionServer};
pub(crate) use e2e_deadline_guard::E2eDeadlineGuardLayer;
pub(crate) use oracle::OracleLayer;
pub(crate) use queue_latency::QueueLatencyLayer;
