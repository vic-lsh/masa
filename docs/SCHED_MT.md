# Multi-thread priority scheduling

Masa normally runs its priority scheduler on Tokio's single-thread runtime.
The `sched_mt` feature enables a multi-thread runtime in which every worker
selects ready tasks from one shared priority-queue abstraction. Local FIFO
queues, the global injection queue, and work stealing are bypassed so that
ready request tasks cannot escape priority scheduling.

## Queue backends

The scheduler supports two queue backends:

| Feature | Backend | Ordering | Intended use |
|---|---|---|---|
| `sched_mt` | `Mutex<BinaryHeap>` | Strict | Simple reference implementation |
| `sched_mt_multiqueue` | `2 × workers` sub-heaps, best-of-two pop | Relaxed | Highest throughput under sustained contention |

`sched_mt_multiqueue` implies `sched_mt`, so applications select only the
top-level backend feature.

The MultiQueue backend is intended for deployments with enough worker threads
to contend on the strict backend's single serialization point. Its relaxed
ordering can occasionally select a nearby-priority task instead of the global
optimum, and its random sampling has higher fixed overhead than the mutex
backend. Use the mutex backend as the low-contention baseline and benchmark the
expected worker count. Use the mutex when strict ordering is required.

The standalone MultiQueue implementation lives in
`libs/multiqueue-prio-queue`.

## Scheduler behavior

`SharedPrioQueue` lives in Tokio's multi-thread scheduler state and is shared
by all workers. Every local or remote task wake-up enters this queue, and each
worker pops its next task from the same abstraction.

Infrastructure tasks (`PriorityHint::infra()`, value `0`) use a small,
dedicated FIFO. An atomic pending flag keeps its mutex off the normal request
hot path while ensuring workers drain infrastructure work before user tasks.
User tasks use the selected heap backend.

The MultiQueue fast path samples two sub-heaps. If both sampled heaps are
empty, the scheduler performs an exhaustive `pop_any` scan before concluding
that it observed no work. This prevents sparse, already-queued work from being
missed before parking without adding a global occupancy counter to every push
and pop.

## Application integration

The application feature must propagate the backend to `masa`, which propagates
it to Tokio:

```toml
[features]
sched_mt = ["masa/sched_mt"]
sched_mt_multiqueue = ["sched_mt", "masa/sched_mt_multiqueue"]
```

Service entry points select Tokio's multi-thread runtime when `sched_mt` is
enabled and retain the current-thread runtime otherwise. Hotel and synthetic
services construct the runtime through `app_utils::runtime::block_on`, which
sizes the worker pool from `CPUS_PER_REPLICA`. The experiment runner passes the
same value to the container CPU quota, preventing worker oversubscription.

For hotel experiments, set the top-level `cpus_per_replica` field in
`hotel.json`; omitting it defaults both the container quota and Tokio worker
count to 4:

```json
{
  "cpus_per_replica": 16
}
```

Example builds:

```bash
cargo build -p hotel --features sched_mt_multiqueue --release
cargo build -p hotel --features "sched_mt_multiqueue,abort_slo" --release
```

## Performance validation

The queue backends were evaluated on the Hotel workload on 2026-07-28. Each
service had a Docker CPU quota of 16 and a Tokio runtime with 16 worker
threads. The load generator sent an equal mix of Search and Reservation
requests with 200 ms and 100 ms SLOs, respectively.

- 3 independent repetitions
- 25-second warmup and 60-second measurement at each load
- 500–4,000 offered requests per second
- no abort or admission-control modifier

Mean SLO-goodput across the three repetitions was:

| Offered RPS | Mutex | Aggregator prototype | MultiQueue |
|---:|---:|---:|---:|
| 2,000 | 1,993 | 1,991 | **1,996** |
| 2,500 | 2,319 | 2,341 | **2,427** |
| 3,000 | 1,710 | 1,656 | **2,569** |
| 3,500 | 531 | 732 | **1,685** |
| 4,000 | 375 | **379** | 370 |

All three repeats showed the same MultiQueue advantage from 2,500 through
3,500 RPS. At 3,000 RPS it improved goodput by 50.2% over the mutex and 55.2%
over the aggregator prototype, while reducing aggregate p99 latency from about
301 ms to 230 ms. Its peak goodput was 2,569 RPS, 9.7% above the best strict
backend's peak.

The crossover begins near 2,500 RPS for this 16-worker configuration. Below
that point, the backends are effectively tied at the application level. At
4,000 RPS all three collapse to roughly 9% SLO attainment; a queue backend
cannot replace overload control. The strict aggregator prototype did not show
a consistent advantage over the mutex and was removed from the implementation.

## Validation

The MultiQueue crate has concurrent exact-once tests, and `scripts/check.sh`
covers both backends with and without `abort_slo`:

```bash
cargo test -p multiqueue-prio-queue
./scripts/check.sh
```
