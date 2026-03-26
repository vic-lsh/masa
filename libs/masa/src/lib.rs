pub use masa_core::{
    time_now, Context, ContextBuilder, FutureSpan, LatencyDistribution, MethodId, PriorityHint,
    TAILCLIPPER,
};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// Utility function to create a Masa Context.
///
/// This handles:
/// 1. Generating a unique request ID (process-local)
/// 2. Capturing the current time as start time
/// 3. Calculating deadline based on SLO
/// 4. Calculating priority hint based on the active scheduling policy (FIFO/Global vs Oldest)
pub fn create_context(api: &str, slo: Duration) -> Context {
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let slo_us = slo.as_micros() as u64;
    let start_at = time_now();
    let deadline = start_at + slo_us;

    // Priority logic:
    // If TAILCLIPPER is enabled, priority is based on arrival time (start_at).
    // Otherwise (sched_fifo, sched_prio, est_abort), priority is based on deadline.
    // Note: TAILCLIPPER is a const bool exported by masa_core based on compile features.
    let prio_hint = if masa_core::TAILCLIPPER {
        start_at
    } else {
        deadline
    };

    #[allow(unused_mut)]
    let mut builder = ContextBuilder::new(api, request_id)
        .slo(slo_us)
        .gateway_entry(start_at)
        .deadline(deadline)
        .prio_hint(PriorityHint::new(prio_hint));

    builder.build()
}

/// Try to create a Masa context, checking the client-side Rajomon token bucket first.
/// Returns None if the client-side rate limiter rejects the request.
#[cfg(feature = "ac_rajomon")]
pub fn try_create_context(api: &str, slo: std::time::Duration) -> Option<Context> {
    use tonic::masa::context::rajomon::CLIENT_TOKEN_BUCKET;

    let method = tonic::CowGrpcMethod::new("", api.to_string());
    tonic::masa::context::rajomon::ClientTokenBucket::ensure_worker_started();
    let tokens = CLIENT_TOKEN_BUCKET.try_acquire(&method)?;

    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let slo_us = slo.as_micros() as u64;
    let start_at = time_now();
    let deadline = start_at + slo_us;

    let prio_hint = if masa_core::TAILCLIPPER {
        start_at
    } else {
        deadline
    };

    Some(
        ContextBuilder::new(api, request_id)
            .slo(slo_us)
            .gateway_entry(start_at)
            .deadline(deadline)
            .prio_hint(PriorityHint::new(prio_hint))
            .tokens(tokens)
            .build(),
    )
}

/// Utility function to create and attach a Masa Context to a Request.
pub fn attach_context<T>(req: &mut tonic::Request<T>, api: &str, slo: Duration) {
    let ctx = create_context(api, slo);
    req.metadata_mut().insert_ctx("ctx", &ctx);
}
