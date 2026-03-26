// Admission control module.
//
// Provides a common `AcHandler` trait for per-request admission control,
// with compile-time selection of the active implementation via feature flags.
// The two admission control strategies (Rajomon token-based and EST
// compute-budget-based) are mutually exclusive.

use tonic::{CowGrpcMethod, Response, Status};
use masa_core::Context;

#[cfg(feature = "ac_rajomon")]
pub mod rajomon;

#[cfg(feature = "ac_est")]
pub(crate) mod est;

#[cfg(feature = "est")]
pub(crate) mod predictive_ac;

mod noop;

/// The active admission control handler, selected at compile time.
///
/// - `ac_rajomon`: Token-based admission control (Rajomon).
/// - `ac_est`: Compute-budget admission control using latency estimates.
/// - Neither: No-op (all requests admitted).
#[cfg(feature = "ac_rajomon")]
pub(crate) type ActiveAcHandler = rajomon::RajomonHandler;
#[cfg(all(feature = "ac_est", not(feature = "ac_rajomon")))]
pub(crate) type ActiveAcHandler = est::EstAcHandler;
#[cfg(not(any(feature = "ac_rajomon", feature = "ac_est")))]
pub(crate) type ActiveAcHandler = noop::NoopAcHandler;

/// Per-request admission control handler.
///
/// Each request creates one `AcHandler` instance (via [`AcHandler::new`]),
/// which is embedded in [`BaseHookState`](super::base::BaseHookState) and
/// called at various points during the request lifecycle. Implementations
/// are selected at compile time via the [`ActiveAcHandler`] type alias.
///
/// All methods except [`new`](AcHandler::new) have default no-op
/// implementations, so strategies only override what they need.
pub(crate) trait AcHandler: Send + Sync + std::fmt::Debug {
    /// Construct a handler for an incoming request on `method`.
    ///
    /// Called once in `BaseHookState::new()` before any other hook fires.
    fn new(method: CowGrpcMethod) -> Self;

    /// Inbound admission check, called immediately after construction.
    ///
    /// May inspect and mutate `ctx` (e.g., to read/deduct tokens).
    /// If the request should be rejected, the implementation records that
    /// internally and [`drop_status`](AcHandler::drop_status) will return
    /// the error on subsequent calls.
    fn check_inbound(&mut self, _ctx: &mut Context) {}

    /// If this request was marked for rejection by [`check_inbound`](AcHandler::check_inbound),
    /// returns the `Status` error to send back. Otherwise returns `None`.
    ///
    /// Called from `BaseHookState::check_guards()` on every poll and
    /// before child RPCs.
    fn drop_status(&self) -> Option<Status> {
        None
    }

    /// Check whether an outbound child RPC to `child_method` should be
    /// allowed under the current admission budget.
    ///
    /// Called from `before_child_rpc` after guard checks pass.
    /// Returns `Err(Status)` to reject the child RPC.
    fn check_outbound(&self, _child_method: &CowGrpcMethod, _ctx: &Context) -> Result<(), Status> {
        Ok(())
    }

    /// Record per-poll queue delay signal for congestion detection.
    ///
    /// Called from `BaseHookState::track_poll()` after guard checks pass
    /// in `before_poll`.
    fn track_queue_delay(&self) {}

    /// Learn from a child RPC response (e.g., cache downstream prices).
    ///
    /// Called from `BaseHookState::update_after_child()` after each child
    /// RPC completes.
    fn on_child_response(
        &self,
        _child_method: &CowGrpcMethod,
        _metadata: &tonic::metadata::MetadataMap,
    ) {
    }

    /// Inject admission-control metadata into the outgoing response
    /// (e.g., price headers for upstream callers).
    ///
    /// Called from `BaseHookState::finalize()` before the response is sent.
    fn inject_response_metadata<T>(&self, _result: &mut Result<Response<T>, Status>) {}

    /// Remaining token/budget to propagate to child request contexts.
    ///
    /// Used in `ContextBuilder` when constructing the child's `Context`.
    /// Rajomon returns the decremented token budget; other strategies
    /// return the default (100).
    fn remaining_tokens(&self) -> u64 {
        100
    }
}
