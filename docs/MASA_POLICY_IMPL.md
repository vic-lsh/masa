# Masa Implementation Details

This document explains the implementation of Masa's dynamic RPC prioritization system. It covers the data flow from client to server, the libraries involved (`masa`, `tonic`, `hyper`, `tokio`), and how feature flags control the scheduling policies.

## 1. Feature Flags and Build Configuration

Masa uses Rust feature flags to select the scheduling policy at compile time. These flags are defined in `libs/masa/Cargo.toml` (the facade) and propagated to `libs/masa-core`, `libs/tonic` and `libs/tokio`.

Key feature flags include:
- `fifo`: First-In-First-Out ordering (baseline).
- `prio_global`: Priority based on end-to-end deadline (global clock).
- `prio_oldest`: Priority based on request arrival time (oldest first).
- `prio_local`: Priority based on local deadlines.
- `early`: Enables "Early Return" to drop requests that have already missed their deadline.

When a specific feature flag (e.g., `prio_global`) is enabled, it activates corresponding conditional compilation modules (`#[cfg(feature = "...")]`) across the modified libraries.

### Feature Flag Propagation

Features propagate from application crates through a dependency chain:

```
Application Cargo.toml (e.g., apps/hotel --features prio_global)
  └─ libs/tonic/tonic/Cargo.toml:  prio_global = ["masa/prio_global", "tokio/prio_global"]
       ├─ libs/masa/Cargo.toml:    prio_global = []   (forwards to masa-core, tokio, tonic)
├─ libs/masa-core/Cargo.toml: prio_global = []   (sets cfg flag)
       └─ libs/tokio/tokio/Cargo.toml: prio_global = ["masa/prio_global"]
```

The root `Cargo.toml` `[patch.crates-io]` section replaces 8 upstream crates (`tokio`, `tokio-util`, `tokio-stream`, `tokio-test`, `tokio-macros`, `hyper`, `tower`, `tower-service`, `tower-layer`) with local modified versions. All must be built from local copies.

### `DefaultMasaHooks` Selection

The `DefaultMasaHooks` type alias (in `libs/tonic/tonic/src/masa/context/mod.rs`) is resolved by feature flag **precedence**. When multiple flags are enabled, the first match wins:

1. `prio_local` → `LocalDeadlinePolicy`
2. `prio_oldest` → `PrioOldest`
3. `prio_global` → `QueueGlobal`
4. `fifo` + `early` → `Fifo`
5. `fifo` (without `early`) → `NoopMasaHooks`
6. Default (no features) → `NoopMasaHooks`

Each flag also selects the corresponding tokio queue implementation (see Section 5).

### Compile-Time Constants

In `libs/masa-core/src/flag.rs`, each feature flag is exposed as a `const bool`:
```
pub const PRIO_GLOBAL: bool = cfg!(feature = "prio_global");
pub const EARLY_RETURN: bool = cfg!(feature = "early");
// ... etc.
```
These allow `if PRIO_GLOBAL { ... }` branches to be optimized away by the compiler when the flag is off.

## 2. Core Data Structures (`libs/masa-core`)

The `masa` crate defines the fundamental types shared across the system.

**Important**: All timestamps, deadlines, SLOs, and latency values throughout the system are in **microseconds** (not milliseconds). `time_now()` in `libs/masa-core/src/timing.rs` returns `SystemTime::now().as_micros() as u64`.

### `Context` and `ContextBuilder`

`Context` carries metadata for a request:
*   `api`: Service/method identifier (`String`).
*   `request_id`: Unique identifier (`u64`).
*   `slo`: Service Level Objective latency (microseconds).
*   `gateway_entry`: Timestamp when the request entered the system (microseconds since UNIX epoch).
*   `deadline`: The computed deadline for this RPC hop (microseconds since UNIX epoch). Not necessarily the end-to-end deadline.
*   `prio_hint`: The priority value used by the scheduler.
*   `frontend_elapse`: Optional elapsed time at frontend.

`Context` also provides `e2e_deadline()`, computed as `gateway_entry + slo`, which is the absolute end-to-end deadline.

`ContextBuilder` creates `Context` instances. If no explicit `prio_hint` is provided, it defaults to `PriorityHint::new(deadline)` — using the per-hop deadline as the priority value.

Serialization:
*   `to_json()` / `from_json()`: JSON format (used for logging/debugging).
*   `to_header_string()` / `from_header_string()`: **Bincode + base64** format (used for HTTP/2 header transport — compact binary, not human-readable).

### `PriorityHint`

A wrapper around `u64`:
*   Lower values indicate **higher** priority.
*   `PriorityHint::infra()` (value 0) is reserved for infrastructure tasks (highest priority).
*   Implements `Ord` such that `BinaryHeap` (max-heap) pops smaller values first (reversed ordering).
*   `Default` returns `infra()`.

### `LatencyEstimator`

A trait with two implementations, used by the `prio_local` policy to estimate child RPC latencies:

*   **`LatencyRms`** (`latency_estimator/rms.rs`): Tracks Root Mean Square of observed latencies. Uses integer square root (Newton's method) to avoid floating-point. Batches updates every N samples (default 512) to amortize cost. `estimate()` returns the cached RMS regardless of the percentile parameter.
*   **`LatencyDistribution`** (`latency_estimator/histogram.rs`): Double-buffered histogram. Collects samples into a current buffer; when it reaches capacity, merges with the previous buffer, sorts, and extracts 100 percentiles. `estimate(p)` returns the p-th percentile.

### `FutureSpan`

Instrumentation metadata for tracing variants:
```
pub enum FutureSpan {
    Compute(u64),      // CPU-bound execution time
    LocalBlock(u64),   // Blocked on local I/O
    ChildBlock(u64),   // Blocked waiting for child RPC
    Queueing(u64),     // Time in scheduler queue
}
```
Serialized to JSON in the `X-Latency-Traces` response header by the `Tracing` policy variant.

## 3. Client-Side Logic (`libs/tonic`)

When a service (acting as a client) sends an RPC to a downstream service, the policy logic is handled by `MasaHooks`.

### Three-Level Hook Architecture

`MasaHooks` (defined in `libs/tonic/tonic/src/masa/context/mod.rs`) is the central trait that associates three context types:

```
pub trait MasaHooks: Send + Sync + 'static {
    type ServerContext: ServerHooks;
    type ChildContext: ClientHooks;
    type ParentContext: ParentHooks<Self::ChildContext, Self::ServerContext>;
}
```

The three levels have different lifetimes and thread-safety requirements:

*   **`ServerHooks`** (`ServerContext`): Created **once per service**. Holds service-wide state (e.g., latency distributions for the `prio_local` policy). Thread-safe (`Send + Sync`).
*   **`ParentHooks`** (`ParentContext`): Created **once per incoming request**. Manages deadline propagation, early return checks, and queue latency tracking. Thread-safe (`Send + Sync`) since it is accessed from both the handler task and child RPC tasks.
*   **`ClientHooks`** (`ChildContext`): Created **once per outgoing RPC call**. Tracks per-call timing. Not thread-safe (accessed only on the calling task).

### Hook Execution Order

For a complete request lifecycle:
1.  `ParentContext::begin()` — server creates context from incoming request.
2.  `before_poll()` — before each poll of the handler future.
3.  `before_child_rpc()` — before making an outgoing RPC (modifies request headers).
4.  `ChildContext::before_send()` — last chance before wire.
5.  [child RPC executes]
6.  `ChildContext::after_recv()` — process child response.
7.  `after_child_rpc()` — parent hooks process child response.
8.  `after_poll()` — after each poll of the handler future.
9.  `finalize_before_serialization()` — after handler completes, before serializing response.
10. `finalize_after_serialization()` — after response is serialized (e.g., inject `x-queue-latency` header).

### Policy Implementations
Different modules implement `MasaHooks` based on the active feature flag:
*   **`QueueGlobal`** (for `prio_global`): In `before_child_rpc`, it calculates the deadline and priority for the child request and injects a `ctx` header. Tracks queue latency via `QueueLatencyTracker`.
*   **`PrioOldest`** (for `prio_oldest`): Like `QueueGlobal`, but the priority hint is the request creation time (older requests = higher priority), implementing the TailClipper approach.
*   **`LocalDeadlinePolicy`** (for `prio_local`): Computes local deadlines by subtracting estimated remaining processing time from the parent deadline. Maintains per-method-pair `LatencyRms` estimators. Only works for applications with a known call graph (currently `hotel`).
    *   Optional modifier: enabling `prio_local_transform` applies a monotone transform to the remaining estimate (`w' = a * w^b`) before deadline/priority computation. The constants are defined in `libs/tonic/tonic/src/masa/context/local/local.rs`.
*   **Fifo**: Passes through deadline/priority. Handles `early` return checks if the `early` feature is also enabled.
*   **Global**: Simplified global deadline policy without queue latency tracking (no early return support).
*   **Noop**: No-op hooks. Selected when `fifo` is enabled without `early`, or when no policy feature is active.
### Client Code Generation

`tonic-build` (`libs/tonic/tonic-build/src/client.rs`) generates client stub methods that integrate with the hook architecture. Each generated unary method:
1.  Creates a `ChildContext` via `ClientHooks::new()`.
2.  Calls `parent_ctx.before_child_rpc()` to modify the request (inject `ctx` header) — **only if `enable_parent_rpc_ctx` is set to `true`** in the protobuf build configuration.
3.  Calls `child_ctx.before_send()`.
4.  Executes the RPC.
5.  Calls `child_ctx.after_recv()` and `parent_ctx.after_child_rpc()`.

The parent context is obtained from thread-local storage via `unsafe { tls::client::get_parent_ctx::<M>() }`, which is set by the poll hooks (see Section 6).

### Header Injection
The `Context` is serialized using **bincode** (compact binary format) and **base64-encoded**, then added to the HTTP/2 headers with the key `ctx`. This propagates the deadline and priority information to the next hop. The format is not human-readable; use `Context::to_json()` for debugging.

### Method Name Override

The `x-masa-method-name` header (`libs/tonic/tonic/src/masa/context/mod.rs`) allows overriding the gRPC method name for latency tracking. This is used by applications where a generic endpoint (e.g., `invoke`) handles multiple logical methods (e.g., the synthetic and mssim applications).

## 4. Transport Layer (`libs/hyper`)

Masa modifies `hyper` to be priority-aware on the server side.

### Server-Side Request Handling
In `libs/hyper/src/proto/h2/server.rs`, when `hyper` receives a new HTTP/2 stream (request):
1.  It checks for the `ctx` header.
2.  **If present**: It calls `.to_str().unwrap()`, then `MasaContext::from_header_string()` (base64 decode → bincode deserialize) to extract the `prio_hint`. Note: these `.unwrap()` calls will **panic** on malformed input (see `docs/MASA_IMPROVEMENTS.md`).
3.  It calls `exec.execute_h2stream_with_prio(future, prio)`.
4.  **If absent**: It calls `exec.execute_h2stream(future)`, which defaults to `PriorityHint::infra()` (highest priority, value 0). This means requests without a `ctx` header are treated as infrastructure and always execute first.

### Executor Interface
The `Exec` enum in `libs/hyper/src/common/exec.rs` has three variants:
*   `Exec::Default`: Uses standard `tokio::spawn()` — **ignores priority**.
*   `Exec::Masa`: Uses `tokio::task::spawn_with_prio()` — forwards priority to the scheduler.
*   `Exec::Executor(...)`: Delegates to a custom executor.

The `ConnStreamExec` trait is extended with `execute_h2stream_with_prio` to pass the priority hint from `hyper` to the underlying executor. Each HTTP/2 stream is an independent task competing in the same priority queue.

## 5. Execution and Runtime (`libs/tokio`)

The core scheduling logic resides in a modified version of `tokio`.

**Critical**: The priority scheduler is implemented **only** in the `current_thread` (single-threaded) scheduler. The `multi_thread` scheduler has **no** priority-aware modifications. Applications **must** use `#[tokio::main(flavor = "current_thread")]`. Using `multi_thread` will silently ignore all priorities and revert to FIFO scheduling.

### `spawn_with_prio`
`tokio` exposes a `spawn_with_prio` function (`libs/tokio/tokio/src/task/spawn.rs`). This function accepts a `Future` and a `PriorityHint`.

The standard `tokio::spawn()` calls `spawn_with_prio(future, PriorityHint::infra())` — meaning all non-Masa tasks (connection management, timers, channel workers, etc.) run at the **highest priority** by default. Only request-processing tasks spawned via `execute_h2stream_with_prio` receive a lower (deadline-based) priority.

`spawn_with_prio` also handles **poll hook inheritance**: it clones the calling task's poll hook (if any) and wraps the child future with it, ensuring thread-local context propagation (see Section 6).

### Task Header

Each tokio task header (`libs/tokio/tokio/src/runtime/task/core.rs`) is extended with three Masa-specific fields:
*   `priority: UnsafeCell<PriorityHint>` — the task's scheduling priority.
*   `poll_hook: UnsafeCell<Option<PollHook>>` — optional before/after poll callbacks.
*   `timer: UnsafeCell<TraceTimer>` — queue latency measurement (see below).

### Priority Scheduler

The `current_thread` scheduler's run queue (`libs/tokio/tokio/src/runtime/scheduler/current_thread/queue/`) is replaced with a priority-aware implementation selected at compile time:

| Feature Flag(s) | Queue Type | Behavior |
|---|---|---|
| (none), `fifo` | `FifoQueue` | Standard `VecDeque` — FIFO ordering |
| `prio_global`, `prio_local` | `BinaryHeapQueue` | `std::collections::BinaryHeap` — O(log n) insert, O(1) pop of highest-priority task |
| `prio_oldest` | `BinaryHeapRoundRobinQueue` | Hybrid: binary heap + round-robin `VecDeque` for the top N=6 highest-priority tasks (prevents starvation) |
| Any tracing variant | `TimedQueue<Inner>` wrapper | Wraps the inner queue, calling `set_enqueue_time()` on push and `record_queue_lat()` on pop |

`BinaryHeapRoundRobinQueue` (for `prio_oldest`) additionally supports a dedicated infrastructure queue: if enabled, `PriorityHint::infra()` tasks are routed to a separate FIFO and always popped first.

When the runtime polls for the next task, it selects the one with the highest priority (lowest `PriorityHint` value). Infrastructure tasks (`infra`, value 0) are always prioritized over request processing tasks.

### `TraceTimer` and Queue Latency Measurement

The `TraceTimer` struct in the task header measures how long a task sits in the ready queue:
1.  **On push**: `TimedQueue::push()` records `Instant::now()` in `timer.last_enqueue`.
2.  **On pop**: `TimedQueue::pop()` computes `elapsed = now - last_enqueue` and stores it in `timer.q_lat`.
3.  **On read**: `tokio::task::obtain_task_queue_latency()` reads `timer.q_lat` from the current task header.

This measurement only occurs when a tracing feature (`fifo_queue_tracing`) is enabled, since only then is the `TimedQueue` wrapper compiled in.

## 6. Poll Hooks

Poll hooks are Masa's mechanism for intercepting every `Future::poll` invocation on request handler tasks (and their child tasks). They serve two purposes: (1) enabling Early Return by checking deadlines on each poll, and (2) propagating the parent request context through thread-local storage so that child RPCs issued from any spawned task can discover their parent.

Poll hooks operate at two layers — tonic and tokio — with different responsibilities.

### Tonic Layer: `ParentHooks` and `AbortableFuture`

The `ParentHooks` trait (`libs/tonic/tonic/src/masa/context/mod.rs`) defines `before_poll` and `after_poll` methods on the per-request `ParentContext`:

*   **`before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>>`**: Called before the handler future is polled. Returning `Err(response)` short-circuits the poll and immediately resolves the future with that response.
*   **`after_poll<Ret>(&self, poll: &Poll<...>) -> Result<(), Result<Response<Ret>, Status>>`**: Called after the handler future is polled. Receives the poll result (`Pending` or `Ready`). Can also short-circuit by returning an error response.

In `masa_unary` (`libs/tonic/tonic/src/server/grpc.rs`), the service handler future is wrapped with `AbortableFuture` (`libs/tonic/tonic/src/util.rs`):

```
service.call(request)
    .abortable()
    .before_poll(|| {
        set_parent_ctx(req_ctx.as_ref());   // install context in thread-local
        match req_ctx.before_poll() {
            Ok(()) => None,                 // continue polling
            Err(e) => Some(e),              // abort with early response
        }
    })
    .after_poll(|poll| {
        reset_parent_ctx();                 // clear thread-local
        match req_ctx.after_poll(poll) {
            Ok(()) => None,
            Err(e) => Some(e),
        }
    })
    .build();
```

On every `Future::poll` of the handler, `AbortableFuture` runs the before-hook, polls the inner future, then runs the after-hook. Either hook can replace the poll result, causing the request to resolve immediately.

### Tokio Layer: `PollHook` and Child Task Propagation

When the handler spawns child tasks via `tokio::task::spawn_with_prio`, the parent's poll hook must propagate to those tasks so that thread-local context is available during their execution. This is handled by a low-level `PollHook` struct in tokio (`libs/tokio/tokio/src/runtime/task/poll_hook.rs`).

`PollHook` stores an opaque context pointer and four function pointers:

*   `before_poll(ctx)`: Invoked before the child future is polled.
*   `after_poll(ctx)`: Invoked after the child future is polled.
*   `on_clone(ctx)`: Increments the context's reference count when the hook is cloned to a new child task.
*   `on_destroy(ctx)`: Decrements the reference count when the hook is dropped.

The hook is stored in the tokio task `Header` alongside the task's priority and trace timer. Each task header has a `poll_hook: UnsafeCell<Option<PollHook>>` field.

`spawn_with_prio` (`libs/tokio/tokio/src/task/spawn.rs`) clones the calling task's poll hook and wraps the child future in a `PollHookFuture`:

```
let parent_task_hdr = current_task_header();
let poll_hook = parent_task_hdr.and_then(|h| h.maybe_clone_poll_hook());

let future = async move {
    match poll_hook {
        Some(hook) => future.with_poll_hook(hook).await,
        None => future.await
    }
};
spawn_inner(future, None, priority)
```

`PollHookFuture` (`libs/tokio/tokio/src/runtime/task/poll_hook.rs`) calls `hook.invoke_before_poll()` and `hook.invoke_after_poll()` around every poll of the wrapped future.

### Hook Wiring: Tonic to Tokio

The bridge is `make_child_task_poll_hook` (`libs/tonic/tonic/src/masa/context/runtime/mod.rs`). It converts the tonic-level `ParentContext` (behind an `Arc`) into a tokio `PollHook`:

*   `before_poll`: Sets the parent context in thread-local storage (`set_parent_ctx`), so child RPCs can discover it.
*   `after_poll`: Clears the thread-local (`reset_parent_ctx`), preventing context leaking to unrelated tasks.
*   `on_clone` / `on_destroy`: Manage `Arc` reference counts for the shared `ParentContext`.

This means the tonic `before_poll`/`after_poll` closures (installed on the top-level handler) set and clear the thread-local for the handler task itself, while the tokio `PollHook` does the same for any spawned child tasks.

### Early Return

Early Return uses poll hooks to abort requests that have already missed their deadline, avoiding wasteful computation. It is gated by the compile-time `early` feature flag (`libs/masa-core/src/flag.rs`: `pub const EARLY_RETURN: bool = cfg!(feature = "early")`).

The `EarlyReturnHandler` (`libs/tonic/tonic/src/masa/context/common.rs`) tracks whether a request should be aborted:

*   `check(&self, ctx: &Context) -> bool`: Returns `false` immediately if `EARLY_RETURN` is disabled. Otherwise, compares the current time against `ctx.deadline()`. Once the deadline passes, sets an atomic flag so subsequent checks short-circuit.
*   `issue_error(&self) -> Status`: Returns a `DeadlineExceeded` status with the service and method name.

Policies that support Early Return call `check` in both `before_poll` and `after_poll(Pending)`:

*   **`before_poll`**: Checks deadline before doing any work in this poll cycle. If expired, returns an error response immediately.
*   **`after_poll`**: Only checks when the poll returned `Pending` (the handler is blocked on I/O or a child RPC). If the deadline has passed while waiting, aborts rather than waiting for the next wake-up. When the poll is `Ready`, the request is already done so no check is needed.

Policies with Early Return: `Fifo`, `QueueGlobal`, `PrioOldest`, and `Local`.

### Queue Latency Tracking

Some policies use `before_poll` to accumulate queue latency — the time a task spent in the ready queue before being polled. The `QueueLatencyTracker` (`libs/tonic/tonic/src/masa/context/common.rs`) calls `tokio::task::obtain_task_queue_latency()` during `before_poll` to read the current task's queue wait time from its `TraceTimer` in the task header. This value is accumulated across all polls and child RPC responses (via the `x-queue-latency` response header), then injected into the outgoing response in `finalize_after_serialization`.

Policies with queue latency tracking: `QueueGlobal` and `PrioOldest`.

### Per-Policy Summary

| Policy | `before_poll` | `after_poll` |
|---|---|---|
| `Fifo` | Early return check | Early return check (on `Pending`) |
| `QueueGlobal` | Early return check, queue latency tracking | Early return check (on `Pending`) |
| `PrioOldest` | Early return check, queue latency tracking | Early return check (on `Pending`) |
| `Local` | Early return check | Early return check (on `Pending`) |
| `Global` | Default (no-op) | Default (no-op) |
| `Noop` | No-op | No-op |

## 7. Application Integration

For an application to use Masa's features, it must:

1.  **Compile with Feature Flags**: Select the desired policy (e.g., `--features prio_global`).
2.  **Use `serve_with_masa`**: In the server initialization code (e.g., `main.rs`), the application calls `.serve_with_masa(addr)` instead of the standard `.serve(addr)`.
    *   This configures the `hyper` server to use the `Exec::Masa` executor, ensuring that priorities are passed to `tokio`.
    *   Using `.serve(addr)` will use `Exec::Default`, which calls standard `tokio::spawn()` and **ignores priorities entirely**.
3.  **Runtime Configuration**: **must** use `#[tokio::main(flavor = "current_thread")]`. The priority-aware scheduler is only implemented in the single-threaded runtime. The multi-threaded runtime will silently ignore priorities.

### `LoadBalancedChannel`

Services connect to downstream replicas using `LoadBalancedChannel` (`libs/tonic/tonic/src/transport/masa_channel/mod.rs`). It:
*   Eagerly connects to all replicas on construction.
*   Uses a custom `tower::balance::masa_balance::Balance` for round-robin load balancing.
*   Spawns the internal buffer worker task with `PriorityHint::infra()` (highest priority), ensuring channel infrastructure is never starved by request tasks.

### `x-queue-latency` Response Header

Policies that track queue latency (`QueueGlobal` and `PrioOldest`) propagate accumulated queue wait times in the `x-queue-latency` response header. The `QueueLatencyTracker` aggregates:
*   The current task's queue latency (from `tokio::task::obtain_task_queue_latency()`).
*   Queue latency reported by child RPCs (parsed from their `x-queue-latency` response headers).

The total is injected into the outgoing response in `finalize_after_serialization()`, creating a recursive aggregation of queue latency across the call graph.

## Summary of Data Flow

1.  **Origin**: Request starts with a default or assigned priority/deadline.
2.  **Client (Upstream)**: `ParentHooks::before_child_rpc` calculates child deadline/priority, serializes `Context` to bincode+base64, and sets the `ctx` HTTP/2 header.
3.  **Network**: Request travels with `ctx` header.
4.  **Server (Downstream) Hyper**: Parses `ctx` header (base64 → bincode → `Context`), extracts `PriorityHint`.
5.  **Server Executor**: `Exec::Masa` calls `tokio::task::spawn_with_prio(handler_future, priority)`.
6.  **Tokio Runtime**: Enqueues task in priority queue (binary heap, round-robin, or FIFO depending on feature flags).
7.  **CPU**: Picks highest priority task (lowest `PriorityHint` value) to execute.
8.  **Each poll cycle**: Tonic's `AbortableFuture` runs `before_poll` (sets thread-local context, checks deadline) → polls handler → runs `after_poll` (clears thread-local, checks deadline if `Pending`). Child tasks inherit a tokio `PollHook` that mirrors the thread-local setup/teardown.
9.  **Response**: `finalize_after_serialization` injects `x-queue-latency` header (if applicable). Response travels back to caller.
