pub use masa_core::{
    time_now, Context, ContextBuilder, FutureSpan, LatencyDistribution, MethodId, PriorityHint,
};

pub use masa_policy;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// The default MasaHooks implementation, selected at compile time.
///
/// - When no scheduling features are enabled: `NoopMasaHooks` (zero overhead).
/// - When any scheduling feature is enabled: `StandardHooks` from `masa-policy`.
#[cfg(not(any(feature = "sched_fifo", feature = "sched_slo", feature = "sched_tailclipper")))]
pub type DefaultMasaHooks = tonic::masa::noop::NoopMasaHooks;

#[cfg(any(feature = "sched_fifo", feature = "sched_slo", feature = "sched_tailclipper"))]
pub type DefaultMasaHooks = masa_policy::StandardHooks;

/// Utility function to create a Masa Context.
///
/// This handles:
/// 1. Generating a unique request ID (process-local)
/// 2. Capturing the current time as start time
/// 3. Calculating deadline based on SLO
/// 4. Deriving priority hint from the compile-time scheduling policy
pub fn create_context(api: &str, slo: Duration) -> Context {
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let slo_us = slo.as_micros() as u64;
    let start_at = time_now();
    let deadline = start_at + slo_us;

    ContextBuilder::new(api, request_id)
        .slo(slo_us)
        .gateway_entry(start_at)
        .deadline(deadline)
        .build()
}

/// Try to create a Masa context, checking the client-side Rajomon token bucket first.
/// Returns None if the client-side rate limiter rejects the request.
#[cfg(feature = "ac_rajomon")]
pub fn try_create_context(api: &str, slo: std::time::Duration) -> Option<Context> {
    use masa_policy::ac::rajomon::CLIENT_TOKEN_BUCKET;

    let method = tonic::CowGrpcMethod::new("", api.to_string());
    masa_policy::ac::rajomon::ClientTokenBucket::ensure_worker_started();
    let tokens = CLIENT_TOKEN_BUCKET.try_acquire(&method)?;

    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let slo_us = slo.as_micros() as u64;
    let start_at = time_now();
    let deadline = start_at + slo_us;

    Some(
        ContextBuilder::new(api, request_id)
            .slo(slo_us)
            .gateway_entry(start_at)
            .deadline(deadline)
            .tokens(tokens)
            .build(),
    )
}

/// Utility function to create and attach a Masa Context to a Request.
pub fn attach_context<T>(req: &mut tonic::Request<T>, api: &str, slo: Duration) {
    let ctx = create_context(api, slo);
    req.metadata_mut().insert_ctx("ctx", &ctx);
}
