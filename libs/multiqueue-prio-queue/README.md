# MultiQueue priority queue

A *k*-relaxed concurrent priority queue for Rust, implementing the
**MultiQueue** algorithm from:

> Rihani, Sanders, Schulz — *"MultiQueues: Simpler, Faster, and Better
> Relaxed Concurrent Priority Queues"*, arXiv:1411.1209 (2014)

## How it works

The queue maintains `num_queues` independent `BinaryHeap<T>` sub-queues, each
guarded by its own `Mutex` (default: `2 × num_threads`).

- **Push** — pick a random sub-queue; if its lock is contended, re-sample
  immediately. O(1) expected attempts when `num_queues > num_threads`.
- **Pop** — sample **two** distinct sub-queues; acquire both locks in
  ascending index order (deadlock-free); pop the one with the higher-priority
  top.

Distributing locks across many sub-queues eliminates the single-lock
bottleneck of a global priority queue. The trade-off is **relaxed ordering**:
`pop` returns the best element *visible from two sampled sub-queues*, not the
guaranteed global maximum. The expected rank error is O(*c*) where
*c* = `num_queues / num_threads`.

### No global serialization point

There is **no global counter or shared aggregate state** on the push/pop hot
path — that would reintroduce the very single-contended-cache-line bottleneck
a MultiQueue exists to avoid. Each sub-queue tracks its own length in an
`AtomicUsize` updated *under that sub-queue's lock*, so it is only ever written
by threads already operating on that sub-queue. `len()` and `is_empty()` read
those per-sub-queue counters with an O(`num_queues`) lock-free scan; they are
approximate and intended for cold-path decisions (e.g. "should this consumer
sleep?"), not the hot path.

## When to use this

Use this crate when **many threads continuously push and pop**, contention on
a single `Mutex<BinaryHeap>` is the bottleneck, and occasional out-of-order
pops are acceptable (for example, deadline scheduling where a slightly stale
priority sometimes wins). Keep the mutex heap when strict ordering or low
fixed overhead matters more than scalability under contention.

## Usage

```toml
[dependencies]
multiqueue-prio-queue = { path = "libs/multiqueue-prio-queue" }
```

```rust
use multiqueue_prio_queue::MultiQueue;
use std::cmp::Reverse;
use std::sync::Arc;
use std::thread;

// Min-heap: smallest value = highest priority.
let q = Arc::new(MultiQueue::<Reverse<u32>>::with_threads(4));

let producers: Vec<_> = (0..4u32).map(|i| {
    let q = Arc::clone(&q);
    thread::spawn(move || {
        for j in 0..25 { q.push(Reverse(i * 25 + j)); }
    })
}).collect();
for p in producers { p.join().unwrap(); }

// `pop` may return None if both sampled sub-queues happen to be empty.
// `pop_any` is the exhaustive fallback that scans every sub-queue before
// reporting that it observed no work.
let mut count = 0;
while count < 100 {
    if q.pop().or_else(|| q.pop_any()).is_some() {
        count += 1;
    }
}
```

## Important: `None` ≠ empty

`pop` returns `None` when **both sampled sub-queues are empty**, even if
other sub-queues contain items. This is intentional (the paper's *relaxed*
semantics). Two ways to handle it in a consumer loop:

- **`pop().or_else(|| pop_any())`** — `pop_any` scans every sub-queue and
  avoids missing work that remains present throughout its scan. It trades
  relaxed ordering for an exhaustive fallback, so it is useful right before
  parking a consumer thread. A concurrent push can still arrive after its
  sub-queue was scanned.
- **Sleep briefly and retry** — acceptable when a small extra latency on the
  last few items is fine and you want to stay fully on the relaxed fast path.

Either way, do **not** busy-spin on bare `pop`.

## License

MIT OR Apache-2.0.
