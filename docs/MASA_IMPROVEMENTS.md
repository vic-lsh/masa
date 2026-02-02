# Masa: Known Limitations and Improvement Opportunities

This document catalogs known limitations, error handling gaps, and potential improvements in the Masa implementation. It is intended as a reference for engineers evaluating robustness, planning hardening work, or investigating production issues.

## 1. Panic-on-Malformed-Input in Context Deserialization

The context deserialization path uses `.unwrap()` at every step. A malformed `ctx` header from any source will panic and crash the connection handler.

**Affected code path** (executed for every incoming HTTP/2 request with a `ctx` header):

| Location | Call | Failure mode |
|---|---|---|
| `libs/hyper/src/proto/h2/server.rs` | `ctx.to_str().unwrap()` | Panics if header contains non-UTF-8 bytes |
| `libs/masa-core/src/context.rs` | `BASE64.decode(s).unwrap()` | Panics if base64 encoding is invalid |
| `libs/masa-core/src/context.rs` | `bincode::deserialize(&bytes).unwrap()` | Panics if binary payload is corrupted or schema-mismatched |
| `libs/tonic/tonic/src/metadata/map.rs` | `.parse().unwrap()` in `insert_ctx` | Panics if serialized value is not valid header content |

**Impact**: A single malformed request can take down a server connection. In a microservice graph, a bug in one service's serialization could cascade.

**Possible improvement**: Replace `.unwrap()` calls with error handling that logs a warning and falls back to `PriorityHint::infra()` (or rejects the request with a gRPC error), similar to the existing fallback when the `ctx` header is absent entirely.

## 2. No Context Format Versioning

The `ctx` header uses bincode serialization of the `Context` struct with no version field. Adding, removing, or reordering fields in `Context` will break deserialization of in-flight requests during a rolling deployment.

**Possible improvement**: Add a version byte prefix to the serialized payload, or use a format with built-in schema evolution (e.g., protobuf).

## 3. Commented-Out Deadline Validation

In `libs/hyper/src/proto/h2/server.rs`, there is a commented-out check:
```
// [TODO:Weixin] Skip if the deadline is already passed.
// if time_now() > prio.value() {
//     panic!("Priority is expired");
// }
```

Requests that have already missed their deadline are still enqueued and processed. The Early Return mechanism (poll hooks) catches these eventually, but only after the task has been spawned and polled at least once.

**Possible improvement**: Drop obviously-expired requests at the hyper layer before spawning a task, reducing scheduler queue pressure.

## 4. Silent Priority Degradation with Multi-Thread Runtime

If an application accidentally uses `#[tokio::main(flavor = "multi_thread")]`, all priorities are silently ignored — the multi-thread scheduler uses standard work-stealing queues with no priority awareness. There is no runtime warning or compile-time check.

**Possible improvement**: Add a runtime assertion in `spawn_with_prio` that verifies the current runtime is single-threaded, or emit a `tracing::warn!` when a non-infra priority is used on a multi-thread runtime.

## 5. No Logging for Missing Context

When the `ctx` header is absent, hyper falls back to `PriorityHint::infra()` with no log output. This is correct behavior for infrastructure requests (health checks, etc.), but makes it hard to diagnose misconfigured services that accidentally omit context.

**Possible improvement**: Log at `debug` or `trace` level when a request arrives without a `ctx` header, including the request path.

## 6. Thread-Local Storage Uses Raw Pointers

The thread-local context storage (`libs/tonic/tonic/src/masa/context/tls.rs`) stores a `*const ()` raw pointer:
```
thread_local! {
    static PARENT_CTX: std::cell::Cell<*const ()> = std::cell::Cell::new(core::ptr::null());
}
```

Access requires `unsafe` and relies on the invariant that the pointer is valid for the duration of the poll. The poll hooks set and clear this around every poll, but a bug in hook lifecycle management could lead to a use-after-free.

**Current mitigation**: The `PollHook` manages `Arc` reference counts via `on_clone`/`on_destroy` callbacks, keeping the `ParentContext` alive. This is correct but relies on manual ref-counting rather than Rust's type system.

## 7. `LatencyTracker` Panics on Misuse

`LatencyTracker` (`libs/masa-core/src/timing.rs`) panics if `start()` is called twice or `record_latency()` is called without a prior `start()`. These are programming errors rather than runtime conditions, but in a complex async system, ordering bugs could be subtle.

**Possible improvement**: Return `Result` instead of panicking, or use a typestate pattern to make misuse a compile-time error.

## 8. `LatencyRms` Ignores Percentile Parameter

`LatencyRms::estimate(percentile)` ignores the `percentile` argument and always returns the RMS value. This could be surprising to callers expecting percentile-based estimation. The `LatencyEstimator` trait's API suggests percentile support, but only `LatencyDistribution` actually provides it.

## 9. Deprecated Local Policy Variants

`libs/tonic/tonic/src/masa/context/local/` contains two deprecated modules (`local_direct.rs`, `local_indirect.rs`) alongside the active `local.rs`. These are marked with doc comments but are still compiled. They add code surface area and could cause confusion.

**Possible improvement**: Remove deprecated modules or gate them behind a `deprecated` feature flag.
