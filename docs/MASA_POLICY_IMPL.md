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

## 6. Application Integration

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
