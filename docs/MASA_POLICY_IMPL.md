di# Masa Implementation Details

This document explains the implementation of Masa's dynamic RPC prioritization system. It covers the data flow from client to server, the libraries involved (`masa`, `tonic`, `hyper`, `tokio`), and how feature flags control the scheduling policies.

## 1. Feature Flags and Build Configuration

Masa uses Rust feature flags to select the scheduling policy at compile time. These flags are defined in `libs/masa/Cargo.toml` and propagated through `libs/tonic` and `libs/tokio`.

Key feature flags include:
- `fifo`: First-In-First-Out ordering (baseline).
- `prio_global`: Priority based on end-to-end deadline (global clock).
- `prio_oldest`: Priority based on request arrival time (oldest first).
- `prio_local`: Priority based on local deadlines.
- `early`: Enables "Early Return" to drop requests that have already missed their deadline.

When a specific feature flag (e.g., `prio_global`) is enabled, it activates corresponding conditional compilation modules (`#[cfg(feature = "...")]`) across the modified libraries.

## 2. Core Data Structures (`libs/masa`)

The `masa` crate defines the fundamental types shared across the system:

*   **`Context`**: Carries metadata for a request, including:
    *   `request_id`: Unique identifier.
    *   `slo`: Service Level Objective latency.
    *   `gateway_entry`: Timestamp when the request entered the system (for global policies).
    *   `deadline`: The computed deadline for the request.
    *   `prio_hint`: The priority value used by the scheduler.
*   **`PriorityHint`**: A wrapper around `u64`.
    *   Lower values indicate **higher** priority.
    *   `PriorityHint::infra()` (value 0) is reserved for infrastructure tasks (highest priority).
    *   Implements `Ord` such that `BinaryHeap` (max-heap) pops smaller values first (reversed ordering).

## 3. Client-Side Logic (`libs/tonic`)

When a service (acting as a client) sends an RPC to a downstream service, the policy logic is handled by `MasaHooks`.

### `MasaHooks` Trait
Defined in `libs/tonic/tonic/src/masa/context/mod.rs`, this trait defines hooks for the RPC lifecycle:
*   `before_child_rpc`: Called before sending a request.
*   `before_poll` / `after_poll`: Called during request processing (used for Early Return checks).

### Policy Implementations
Different modules implement `MasaHooks` based on the active feature flag:
*   **`QueueGlobal`** (for `prio_global`): In `before_child_rpc`, it calculates the deadline and priority for the child request and injects a `ctx` header into the HTTP request.
*   **`Fifo`**: Mostly passes through, but handles `early` return checks if enabled.

### Header Injection
The `Context` is serialized to JSON and added to the HTTP/2 headers with the key `ctx`. This propagates the deadline and priority information to the next hop.

## 4. Transport Layer (`libs/hyper`)

Masa modifies `hyper` to be priority-aware on the server side.

### Server-Side Request Handling
In `libs/hyper/src/proto/h2/server.rs`, when `hyper` receives a new HTTP/2 stream (request):
1.  It checks for the `ctx` header.
2.  **If present**: It deserializes the `MasaContext` and extracts the `prio_hint`.
3.  It calls `exec.execute_h2stream_with_prio(future, prio)`.
4.  **If absent**: It falls back to standard execution (often treated as default/infra priority).

### Executor Interface
The `ConnStreamExec` trait in `hyper` is extended to support `execute_h2stream_with_prio`. This allows `hyper` to pass the priority hint down to the underlying executor.

## 5. Execution and Runtime (`libs/tokio`)

The core scheduling logic resides in a modified version of `tokio`.

### `spawn_with_prio`
`tokio` exposes a `spawn_with_prio` function (enabled by `rt` feature). This function accepts a `Future` and a `PriorityHint`.

### `Exec::Masa`
In `libs/tonic/tonic/src/transport/service/executor.rs` and `libs/hyper/src/common/exec.rs`, the `Exec` enum has a `Masa` variant. When this variant is used:
*   `execute_h2stream_with_prio` calls `tokio::task::spawn_with_prio`.

### Priority Scheduler
The `tokio` runtime (specifically the `current_thread` scheduler, which is often used in Masa applications) is modified to use priority queues:
*   Instead of a single FIFO queue, it maintains tasks in a priority-ordered structure (e.g., `prio_bh` for Binary Heap).
*   When the runtime polls for the next task, it selects the one with the highest priority (lowest `PriorityHint` value).
*   Infrastructure tasks (`infra`) are always prioritized over request processing tasks.

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

Early Return uses poll hooks to abort requests that have already missed their deadline, avoiding wasteful computation. It is gated by the compile-time `early` feature flag (`libs/masa/src/flag.rs`: `pub const EARLY_RETURN: bool = cfg!(feature = "early")`).

The `EarlyReturnHandler` (`libs/tonic/tonic/src/masa/context/common.rs`) tracks whether a request should be aborted:

*   `check(&self, ctx: &Context) -> bool`: Returns `false` immediately if `EARLY_RETURN` is disabled. Otherwise, compares the current time against `ctx.deadline()`. Once the deadline passes, sets an atomic flag so subsequent checks short-circuit.
*   `issue_error(&self) -> Status`: Returns a `DeadlineExceeded` status with the service and method name.

Policies that support Early Return call `check` in both `before_poll` and `after_poll(Pending)`:

*   **`before_poll`**: Checks deadline before doing any work in this poll cycle. If expired, returns an error response immediately.
*   **`after_poll`**: Only checks when the poll returned `Pending` (the handler is blocked on I/O or a child RPC). If the deadline has passed while waiting, aborts rather than waiting for the next wake-up. When the poll is `Ready`, the request is already done so no check is needed.

Policies with Early Return: `Fifo`, `QueueGlobal`, `PrioOldest`, and `Local`.

### Queue Latency Tracking

Some policies use `before_poll` to accumulate queue latency — the time a task spent in the ready queue before being polled. The `QueueLatencyTracker` (`libs/tonic/tonic/src/masa/context/common.rs`) calls `tokio::task::obtain_task_queue_latency()` during `before_poll` to read the current task's queue wait time from its `TraceTimer` in the task header. This value is accumulated across all polls and child RPC responses (via the `x-queue-latency` response header), then injected into the outgoing response in `finalize_after_serialization`.

Policies with queue latency tracking: `QueueGlobal`, `PrioOldest`, and the queue-tracing variants.

### Per-Policy Summary

| Policy | `before_poll` | `after_poll` |
|---|---|---|
| `Fifo` | Early return check | Early return check (on `Pending`) |
| `QueueGlobal` | Early return check, queue latency tracking | Early return check (on `Pending`) |
| `PrioOldest` | Early return check, queue latency tracking | Early return check (on `Pending`) |
| `Local` | Early return check | Early return check (on `Pending`) |
| `Global` | Default (no-op) | Default (no-op) |
| `Noop` | No-op | No-op |
| Tracing variants | Timing instrumentation | Timing instrumentation |

## 7. Application Integration

For an application to use Masa's features, it must:

1.  **Compile with Feature Flags**: Select the desired policy (e.g., `--features prio_global`).
2.  **Use `serve_with_masa`**: In the server initialization code (e.g., `main.rs`), the application calls `.serve_with_masa(addr)` instead of the standard `.serve(addr)`.
    *   This configures the `hyper` server to use the `Exec::Masa` executor, ensuring that priorities are passed to `tokio`.
3.  **Runtime Configuration**: typically use `#[tokio::main(flavor = "current_thread")]` to ensure the priority-aware single-threaded scheduler is used, avoiding the complexities of work-stealing priority in multi-threaded runtimes.

## Summary of Data Flow

1.  **Origin**: Request starts with a default or assigned priority/deadline.
2.  **Client (Upstream)**: `MasaHooks` calculates child deadline/priority and sets `ctx` header.
3.  **Network**: Request travels with `ctx` header.
4.  **Server (Downstream) Hyper**: Parses `ctx` header, extracts `PriorityHint`.
5.  **Server Executor**: Calls `tokio::spawn_with_prio(handler_future, priority)`.
6.  **Tokio Runtime**: Enqueues task in priority queue.
7.  **CPU**: Picks highest priority task to execute.
8.  **Each poll cycle**: Tonic's `AbortableFuture` runs `before_poll` (sets thread-local context, checks deadline) → polls handler → runs `after_poll` (clears thread-local, checks deadline if `Pending`). Child tasks inherit a tokio `PollHook` that mirrors the thread-local setup/teardown.
