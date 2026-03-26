// Zero-cost wrapper for pred_sched-specific scheduling behavior.
//
// This module is only compiled with the `est` feature (`pred_sched` implies `est`).
//
// When `sched_pred` is enabled, PredictiveSchedPolicy performs dynamic reprioritization,
// deadline tightening, and child error propagation. When disabled, all methods
// are no-ops that compile away entirely.

use tonic::{Response, Status};
use masa_core::Context;

/// Zero-cost policy overlay for predictive scheduling behavior.
///
/// This struct has no fields and exists purely to provide feature-gated method
/// implementations that `standard.rs` calls when `est` is enabled.
#[derive(Debug)]
pub(super) struct PredictiveSchedPolicy;

// ── pred_sched ENABLED ───────────────────────────────────────────────────────

#[cfg(feature = "sched_pred")]
impl PredictiveSchedPolicy {
    /// Reprioritize the current task based on remaining time to deadline.
    #[inline]
    pub(super) fn reprioritize(&self, ctx: &Context) {
        let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
        tokio::task::reprioritize(masa_core::PriorityHint::new(remaining));
    }

    /// Compute child deadline and priority hint by tightening the parent
    /// deadline: `d = d_parent - est_remaining`.
    ///
    /// Uses deadline alone as prio_hint (not `deadline - est_child`) to avoid
    /// priority inversions under load: as est_child grows with queuing, newer
    /// requests would get tighter prio_hints than older ones, inverting FIFO order.
    #[inline]
    pub(super) fn child_deadline_and_prio(
        &self,
        ctx: &Context,
        est_remaining: u64,
    ) -> (u64, masa_core::PriorityHint) {
        let d = ctx.deadline().saturating_sub(est_remaining);
        (d, masa_core::PriorityHint::new(d))
    }

    /// Propagate child RPC errors to the parent handler.
    #[inline]
    pub(super) fn check_child_response<T>(
        &self,
        response: &Result<Response<T>, Status>,
    ) -> Result<(), Status> {
        if let Err(status) = response {
            // NOTE: could we avoid cloning here?
            return Err(status.clone());
        }
        Ok(())
    }
}

// ── pred_sched DISABLED ──────────────────────────────────────────────────────

#[cfg(not(feature = "sched_pred"))]
impl PredictiveSchedPolicy {
    #[inline]
    pub(super) fn reprioritize(&self, _ctx: &Context) {}

    #[inline]
    pub(super) fn child_deadline_and_prio(
        &self,
        ctx: &Context,
        _est_remaining: u64,
    ) -> (u64, masa_core::PriorityHint) {
        (ctx.deadline(), ctx.prio_hint())
    }

    #[inline]
    pub(super) fn check_child_response<T>(
        &self,
        _response: &Result<Response<T>, Status>,
    ) -> Result<(), Status> {
        Ok(())
    }
}
