pub use masa_core::{
    time_now, Context, ContextBuilder, FutureSpan, LatencyDistribution, MethodId, PriorityHint,
    ORACLE_CHILD_WORK_US_HEADER, ORACLE_REMAINING_AFTER_US_HEADER,
};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// The default Hooks implementation, selected at compile time by Masa features.
///
/// No scheduling features select `NoopHooks` (zero overhead). Any scheduling
/// feature selects `masa_policy::PolicyHooks` with full scheduling hooks.
#[cfg(not(any(
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper",
    feature = "sched_oracle"
)))]
pub type DefaultHooks = masa_tonic_core::noop::NoopHooks;

/// The default Hooks implementation, selected at compile time by Masa features.
#[cfg(any(
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper",
    feature = "sched_oracle"
))]
pub type DefaultHooks = masa_policy::PolicyHooks;

pub mod transport;

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
    use masa_policy::CLIENT_TOKEN_BUCKET;

    let method = tonic::CowGrpcMethod::new("", api.to_string());
    masa_policy::ClientTokenBucket::ensure_worker_started();
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
    use masa_policy::context_ext::MasaRequestExt;
    let ctx = create_context(api, slo);
    req.set_masa_context(&ctx);
}

/// Update the cached Rajomon price for a method (called when a response header is received).
#[cfg(feature = "ac_rajomon")]
pub fn update_rajomon_price(method: &tonic::CowGrpcMethod, price: u64) {
    masa_policy::CLIENT_TOKEN_BUCKET.update_price(method, price);
}

/// Check the client-side Rajomon bucket and draw a random token count for an
/// outgoing request. Returns `None` when the bucket can't cover the cached
/// price for the method (client-side shed). Use this from loadgens that build
/// their own `Context` (and therefore can't use `try_create_context`).
#[cfg(feature = "ac_rajomon")]
pub fn try_acquire_tokens(api: &str) -> Option<u64> {
    use masa_policy::CLIENT_TOKEN_BUCKET;
    let method = tonic::CowGrpcMethod::new("", api.to_string());
    masa_policy::ClientTokenBucket::ensure_worker_started();
    CLIENT_TOKEN_BUCKET.try_acquire(&method)
}

/// Initial token value for Rajomon admission control (runtime-configurable).
#[cfg(feature = "ac_rajomon")]
pub fn tokens_left_init() -> u64 {
    masa_policy::PolicyParams::global().rajomon.tokens_left_init
}
