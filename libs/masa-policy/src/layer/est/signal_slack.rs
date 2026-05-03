//! signal_slack: soft-deadline signal for predictive admission.
//!
//! Mirrors `abort_slack`'s deadline check, but instead of aborting the
//! request we record a flag that propagates to the ingress admission
//! controller via `ResponseMeta`. Also gates latency-estimator updates so
//! a signal-but-continue request's inflated wallclock does not poison the
//! parent->child estimate.
//!
//! All entry points compile out to no-ops when the `signal_slack` feature
//! is disabled — the `SIGNAL_SLACK` const collapses the `if` guard.

use masa_core::{Context, SIGNAL_SLACK};

use super::state::RequestMetadataTracker;

/// Mark the soft deadline signal on the per-request metadata if the
/// current hop is past its local deadline. Called from
/// `EstimationLayer::{before_poll, after_poll}`.
#[inline]
pub(crate) fn mark_if_late(ctx: &Context, meta: &RequestMetadataTracker) {
    if !SIGNAL_SLACK {
        return;
    }
    let local_deadline = ctx.deadline();
    if local_deadline != 0 && masa_core::time_now() > local_deadline {
        meta.mark_deadline_signal();
    }
}

/// True when this request's subtree tripped a soft deadline signal and the
/// estimator should skip flushing observations from it. Called from
/// `EstimationLayer::finalize`.
#[inline]
pub(crate) fn should_skip_flush(meta: &RequestMetadataTracker) -> bool {
    SIGNAL_SLACK && meta.is_subtree_signaled()
}
