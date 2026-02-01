# Masa Policy Configuration Guide

The Masa system uses a compositional policy framework that allows you to configure the **Queueing Discipline**, **Deadline Strategy**, and **Admission Control** independently via command-line flags.

## CLI Flags

Applications using `app-utils` (like the Hotel services) inherit the following flags:

| Flag | Values | Default | Description |
| :--- | :--- | :--- | :--- |
| `--queue` | `fifo`, `prio`, `prio-oldest` | `fifo` | The scheduling discipline for the Tokio task queue. |
| `--deadline-policy` | `none`, `local`, `global`, `oldest` | `none` | The logic used to calculate deadlines for child RPCs. |
| `--early-return` | (Boolean Flag) | `false` | If present, the service will drop requests that have already exceeded their deadline. |

---

## 1. Queueing Disciplines (`--queue`)

*   **`fifo`**: Standard First-In-First-Out scheduling. Tasks are executed in the order they are spawned.
*   **`prio`**: Priority scheduling using a Binary Heap. Tasks with earlier deadlines are prioritized.
*   **`prio-oldest`**: Priority scheduling with a Round-Robin window for the highest priority tasks. This prevents extreme starvation of tasks with slightly lower priority but similar deadlines.

## 2. Deadline Policies (`--deadline-policy`)

*   **`none`**: No deadline logic is applied. Child RPCs do not inherit or calculate specific deadlines.
*   **`global`**: Propagates the end-to-end deadline. Child RPC deadline = Parent Request deadline.
*   **`local`**: Uses local latency estimation. Child RPC deadline = `Parent Deadline - Estimated Remaining Time`.
*   **`oldest`**: Sets priority based on the absolute start time of the original client request (Request Generation Time).

## 3. Admission Control (`--early-return`)

*   When enabled via `--early-return`, the system checks the current time against the request deadline at several "Hook Points" (e.g., before polling a task, before sending a child RPC).
*   If the deadline has passed, the system returns a `DeadlineExceeded` error immediately, saving CPU cycles by avoiding "zombie" work.

---

## Configuration Examples

### Performance-First (Default)
Standard behavior without Masa overhead.
```bash
./hotel_reservation --config config.json --queue fifo --deadline-policy none
```

### Deadline-Aware Scheduling
Prioritizes tasks by deadline but does not drop late requests.
```bash
./hotel_reservation --config config.json --queue prio --deadline-policy global
```

### Full Masa Stack (Local Estimation + Early Return)
The most advanced configuration: prioritizes by deadline, calculates slack via local estimation, and sheds load when deadlines are missed.
```bash
./hotel_reservation --config config.json \
    --queue prio \
    --deadline-policy local \
    --early-return
```

### Fairness-Optimized
Uses the Round-Robin priority queue to balance fairness and urgency.
```bash
./hotel_reservation --config config.json \
    --queue prio-oldest \
    --deadline-policy global \
    --early-return
```

## How It Works

While these flags are parsed at runtime, the system uses **Rust Generics and Macros** to monomorphize the implementation. When you select a combination of flags, the `launch_masa_server!` macro dispatches execution to a specialized version of your service code compiled for that specific policy. This ensures that the hot path (the request processing loop) has **zero dynamic dispatch overhead**.
```rust
// Internally dispatched as:
run_server::<CompositePolicy<masa::Prio, true, DeadlinePolicyLocal>>(args)
```
