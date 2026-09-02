//! A *k*-relaxed concurrent priority queue implementing the **MultiQueue** from
//! Rihani et al., *"MultiQueues: Simpler, Faster, and Better Relaxed
//! Concurrent Priority Queues"*, arXiv:1411.1209.
//!
//! # Algorithm
//!
//! The queue maintains `num_queues` independent `BinaryHeap<T>` sub-queues,
//! each guarded by its own `Mutex`. The default is `2 × num_threads`
//! sub-queues (use [`MultiQueue::with_threads`]).
//!
//! - **Push** — pick a random sub-queue index and `try_lock` it; if the
//!   lock is contended, re-sample immediately. Expected O(1) attempts when
//!   `num_queues > num_threads`.
//!
//! - **Pop** — sample **two** distinct random sub-queues; acquire both locks
//!   (always in index order to avoid deadlock); pop from the one with the
//!   higher-priority top element. If either lock is contended, release
//!   everything and re-sample.
//!
//! # No global serialization point
//!
//! Crucially, this queue keeps **no global counter or shared aggregate
//! state** on the push/pop hot path. The whole point of a MultiQueue is to
//! avoid the single contended cache line that a `Mutex<BinaryHeap>` (or any
//! globally-counted structure) suffers under many threads. Each sub-queue
//! tracks its own length in an [`AtomicUsize`] updated *under that
//! sub-queue's lock*, so the counter for sub-queue *i* is only ever written
//! by threads already operating on sub-queue *i* — never a global hotspot.
//!
//! [`len`](MultiQueue::len) and [`is_empty`](MultiQueue::is_empty) therefore
//! read those per-sub-queue counters with an O(`num_queues`) lock-free scan.
//! They are *approximate* under concurrency and meant for cold-path decisions
//! (e.g. "should a consumer go to sleep?"), not for the hot path.
//!
//! # Relaxed ordering
//!
//! This queue provides **k-relaxed** semantics: `pop` returns the maximum
//! element *visible from the two sampled sub-queues*, not necessarily the
//! global maximum. The expected rank of the returned element is O(*c*) where
//! *c* = `num_queues / num_threads`.
//!
//! **Important:** [`pop`](MultiQueue::pop) may return `None` even when the
//! queue is non-empty, if both sampled sub-queues happen to be empty. A
//! consumer that must not miss existing work should fall back to
//! [`pop_any`](MultiQueue::pop_any), which scans every sub-queue before
//! reporting that it observed no work:
//!
//! ```rust,no_run
//! use multiqueue_prio_queue::MultiQueue;
//!
//! let q: MultiQueue<u32> = MultiQueue::with_threads(4);
//!
//! // Best-of-2 fast path with an exhaustive fallback.
//! let item = q.pop().or_else(|| q.pop_any());
//! ```
//!
//! # Max-heap semantics
//!
//! `pop` returns the **maximum** element under `T`'s [`Ord`] implementation.
//! For a min-heap, wrap `T` in [`std::cmp::Reverse`]:
//!
//! ```rust
//! use multiqueue_prio_queue::MultiQueue;
//! use std::cmp::Reverse;
//!
//! let q: MultiQueue<Reverse<u32>> = MultiQueue::with_threads(2);
//! q.push(Reverse(10));
//! q.push(Reverse(1));
//! q.push(Reverse(5));
//!
//! // Most likely pops Reverse(1) first, but relaxed ordering means
//! // a higher-indexed sub-queue's top might win on a given call.
//! let _ = q.pop();
//! ```
//!
//! # When to use this
//!
//! - You have **many threads continuously pushing and popping** and contention
//!   on a single lock is the bottleneck.
//! - You can tolerate **O(c) ordering relaxation** — acceptable for deadline
//!   scheduling where a slightly-stale priority occasionally wins.
//! - If you need **strict** ordering, use a mutex-protected
//!   [`BinaryHeap`](std::collections::BinaryHeap) instead.
//!
//! # Example
//!
//! ```rust
//! use multiqueue_prio_queue::MultiQueue;
//! use std::sync::Arc;
//! use std::thread;
//!
//! let q = Arc::new(MultiQueue::<u32>::with_threads(4));
//!
//! // Four concurrent producers.
//! let producers: Vec<_> = (0..4u32).map(|i| {
//!     let q = Arc::clone(&q);
//!     thread::spawn(move || {
//!         for j in 0..25 { q.push(i * 25 + j); }
//!     })
//! }).collect();
//! for p in producers { p.join().unwrap(); }
//!
//! // Drain exhaustively.
//! let mut count = 0;
//! while q.pop_any().is_some() { count += 1; }
//! assert_eq!(count, 100);
//! ```

#![warn(missing_docs)]

use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use rand::Rng;

/// Default number of sub-queues for [`MultiQueue::default()`].
///
/// Sized for up to ~8 concurrent threads with c = 2.
pub const DEFAULT_NUM_QUEUES: usize = 16;

/// A single sub-queue: a lock-guarded heap plus its own length counter.
///
/// The `len` counter is written only while holding `heap`'s lock, so it is
/// never a globally contended cache line — it is touched solely by threads
/// already operating on this sub-queue. Reads for `len` and `is_empty` are
/// lock-free.
struct SubQueue<T: Ord> {
    heap: Mutex<BinaryHeap<T>>,
    len: AtomicUsize,
}

impl<T: Ord> SubQueue<T> {
    fn new() -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::new()),
            len: AtomicUsize::new(0),
        }
    }
}

/// A *k*-relaxed concurrent priority queue.
///
/// See the [crate documentation](crate) for a full description of the
/// algorithm and ordering guarantees.
pub struct MultiQueue<T: Ord> {
    queues: Vec<SubQueue<T>>,
}

// `Vec<SubQueue<T>>` is `Send + Sync` when `T: Send`, so these impls are
// derived automatically by the compiler; no unsafe block required.

impl<T: Ord + Send> Default for MultiQueue<T> {
    /// Creates a `MultiQueue` with [`DEFAULT_NUM_QUEUES`] sub-queues.
    fn default() -> Self {
        Self::new(DEFAULT_NUM_QUEUES)
    }
}

impl<T: Ord + Send> std::fmt::Debug for MultiQueue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiQueue")
            .field("num_queues", &self.queues.len())
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<T: Ord + Send> MultiQueue<T> {
    /// Creates a new empty `MultiQueue` with `num_queues` sub-queues.
    ///
    /// The paper recommends `c × num_threads` sub-queues with `c ≥ 2`.
    /// Increasing `c` reduces contention but increases the expected rank
    /// error of each `pop`.
    ///
    /// # Panics
    ///
    /// Panics if `num_queues < 2` (two sub-queues are required for the
    /// best-of-2 pop strategy).
    pub fn new(num_queues: usize) -> Self {
        assert!(num_queues >= 2, "MultiQueue requires at least 2 sub-queues");
        Self {
            queues: (0..num_queues).map(|_| SubQueue::new()).collect(),
        }
    }

    /// Creates a new empty `MultiQueue` sized for `threads` concurrent
    /// threads, using `2 × threads` sub-queues (the paper's recommended
    /// default of c = 2).
    ///
    /// # Panics
    ///
    /// Panics if `threads == 0`.
    pub fn with_threads(threads: usize) -> Self {
        assert!(threads > 0, "threads must be at least 1");
        Self::new(threads * 2)
    }

    /// Returns the number of sub-queues.
    #[inline]
    pub fn num_queues(&self) -> usize {
        self.queues.len()
    }

    /// Pushes `item` into a randomly selected sub-queue.
    ///
    /// If the selected sub-queue's lock is contended, yields the thread and
    /// re-samples. This is cooperative-friendly for use inside async runtimes.
    pub fn push(&self, item: T) {
        let mut rng = rand::thread_rng();
        let n = self.queues.len();
        loop {
            let i = rng.gen_range(0..n);
            if let Ok(mut g) = self.queues[i].heap.try_lock() {
                g.push(item);
                self.queues[i].len.fetch_add(1, Ordering::AcqRel);
                return;
            }
            std::thread::yield_now();
        }
    }

    /// Removes and returns the maximum element visible from two randomly
    /// sampled sub-queues, or `None` if both sampled sub-queues are empty.
    ///
    /// **Relaxed semantics:** the returned element is the maximum of two
    /// randomly chosen sub-queues, not necessarily the global maximum. The
    /// expected rank of the returned element is O(`num_queues / num_threads`).
    ///
    /// **`None` does not mean empty:** other sub-queues may still contain
    /// items. A consumer that must not miss existing work should fall back to
    /// [`pop_any`](Self::pop_any):
    ///
    /// ```rust,no_run
    /// use multiqueue_prio_queue::MultiQueue;
    ///
    /// let q: MultiQueue<u32> = MultiQueue::with_threads(4);
    /// q.push(42);
    ///
    /// let item = q.pop().or_else(|| q.pop_any());
    /// assert_eq!(item, Some(42));
    /// ```
    pub fn pop(&self) -> Option<T> {
        let mut rng = rand::thread_rng();
        let n = self.queues.len();
        loop {
            // Sample two distinct indices.
            let i = rng.gen_range(0..n);
            let j = {
                let mut j = rng.gen_range(0..n - 1);
                if j >= i {
                    j += 1;
                }
                j
            };

            // Always lock in ascending index order to prevent deadlock.
            let (lo, hi) = if i < j { (i, j) } else { (j, i) };

            let mut g_lo = match self.queues[lo].heap.try_lock() {
                Ok(g) => g,
                Err(_) => {
                    std::thread::yield_now();
                    continue;
                }
            };
            let mut g_hi = match self.queues[hi].heap.try_lock() {
                Ok(g) => g,
                Err(_) => {
                    drop(g_lo);
                    std::thread::yield_now();
                    continue;
                }
            };

            // Pop from whichever of the two locked sub-queues has the better
            // top, returning the index we popped from so we can decrement its
            // length counter.
            let (result, idx) = match (g_lo.peek(), g_hi.peek()) {
                (None, None) => return None,
                (Some(_), None) => (g_lo.pop(), lo),
                (None, Some(_)) => (g_hi.pop(), hi),
                (Some(a), Some(b)) => {
                    // BinaryHeap is a max-heap; with reversed Ord (lower deadline
                    // = higher priority = "greater"), peek() surfaces the most
                    // urgent element.
                    if a >= b {
                        (g_lo.pop(), lo)
                    } else {
                        (g_hi.pop(), hi)
                    }
                }
            };

            if result.is_some() {
                self.queues[idx].len.fetch_sub(1, Ordering::AcqRel);
            }
            return result;
        }
    }

    /// Removes and returns an element from the queue, scanning **every**
    /// sub-queue before returning `None`.
    ///
    /// This is the exhaustive fallback for [`pop`](Self::pop)'s relaxed
    /// best-of-2 sampling. Because it inspects sub-queues in index order and
    /// returns the first element found, it does **not** honor the global
    /// priority order — it trades ordering for a guarantee of progress. Use
    /// it when a missed element is unacceptable (e.g. before parking a worker
    /// thread), typically as `q.pop().or_else(|| q.pop_any())`: the relaxed
    /// `pop` handles the common, high-occupancy case cheaply, and `pop_any`
    /// only runs when sampling came up empty.
    ///
    /// Like any observation of a concurrent queue, the result is immediately
    /// stale: a producer can push to an already-scanned sub-queue before this
    /// method returns.
    pub fn pop_any(&self) -> Option<T> {
        // First pass: never block. Most callers hit an element immediately.
        for q in &self.queues {
            if let Ok(mut g) = q.heap.try_lock() {
                if let Some(v) = g.pop() {
                    q.len.fetch_sub(1, Ordering::AcqRel);
                    return Some(v);
                }
            }
        }
        // Second pass: block on each lock so a transiently-contended sub-queue
        // cannot make us miss work that remains in place throughout the scan.
        for q in &self.queues {
            let mut g = q.heap.lock().unwrap();
            if let Some(v) = g.pop() {
                q.len.fetch_sub(1, Ordering::AcqRel);
                return Some(v);
            }
        }
        None
    }

    /// Returns an **approximate** element count.
    ///
    /// Computed as an O(`num_queues`) lock-free scan summing each sub-queue's
    /// own length counter. There is no global counter, so the result may lag
    /// slightly behind in-flight operations under concurrent access.
    #[inline]
    pub fn len(&self) -> usize {
        self.queues
            .iter()
            .map(|q| q.len.load(Ordering::Acquire))
            .sum()
    }

    /// Returns `true` if every sub-queue appeared empty during an
    /// O(`num_queues`) lock-free scan.
    ///
    /// Subject to the same approximation as [`len`](Self::len). A `true`
    /// result does **not** guarantee that [`pop`](Self::pop) will return
    /// `None` — a concurrent push may have arrived between the scan and the
    /// pop.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queues
            .iter()
            .all(|q| q.len.load(Ordering::Acquire) == 0)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Reverse;
    use std::collections::HashSet;
    use std::sync::atomic::Ordering::Relaxed;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn drain(q: &MultiQueue<usize>) -> Vec<usize> {
        let mut out = Vec::new();
        while let Some(v) = q.pop_any() {
            out.push(v);
        }
        out
    }

    #[test]
    fn single_thread_all_values_present() {
        let q = MultiQueue::new(4);
        for i in 0..20usize {
            q.push(i);
        }
        let mut out = drain(&q);
        out.sort_unstable();
        assert_eq!(out, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn min_heap_via_reverse() {
        // The multiqueue has relaxed ordering, so we can only guarantee that
        // all values are eventually returned, not that they arrive in order.
        let q: MultiQueue<Reverse<u32>> = MultiQueue::new(4);
        let input = [3u32, 1, 4, 1, 5];
        for &v in &input {
            q.push(Reverse(v));
        }
        let mut out = Vec::new();
        while let Some(Reverse(v)) = q.pop_any() {
            out.push(v);
        }
        out.sort_unstable();
        let mut expected: Vec<u32> = input.to_vec();
        expected.sort_unstable();
        assert_eq!(out, expected);
    }

    #[test]
    fn empty_queue_behaviour() {
        let q: MultiQueue<u32> = MultiQueue::with_threads(2);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        // pop on empty returns None (relaxed: both sampled sub-queues are empty)
        assert_eq!(q.pop(), None);
        // pop_any on empty also returns None (exhaustive scan).
        assert_eq!(q.pop_any(), None);
        q.push(7);
        assert!(!q.is_empty());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn pop_any_finds_lone_element() {
        // With many sub-queues and a single element, relaxed `pop` will
        // usually miss, but `pop_any` must always find it.
        let q: MultiQueue<u32> = MultiQueue::new(64);
        q.push(99);
        assert_eq!(q.pop_any(), Some(99));
        assert!(q.is_empty());
    }

    #[test]
    fn with_threads_sets_num_queues() {
        let q: MultiQueue<u32> = MultiQueue::with_threads(6);
        assert_eq!(q.num_queues(), 12);
    }

    #[test]
    fn default_has_correct_num_queues() {
        let q: MultiQueue<u32> = MultiQueue::default();
        assert_eq!(q.num_queues(), DEFAULT_NUM_QUEUES);
    }

    #[test]
    fn concurrent_producers_all_values_present() {
        const N: usize = 2000;
        const THREADS: usize = 8;
        let q = Arc::new(MultiQueue::with_threads(THREADS));

        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let q = Arc::clone(&q);
                thread::spawn(move || {
                    for i in 0..(N / THREADS) {
                        q.push(t * (N / THREADS) + i);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(q.len(), N);
        let mut seen = HashSet::with_capacity(N);
        while let Some(v) = q.pop_any() {
            seen.insert(v);
        }
        assert_eq!(seen.len(), N);
        for i in 0..N {
            assert!(seen.contains(&i), "missing value {i}");
        }
    }

    #[test]
    fn concurrent_producers_and_consumers() {
        const ITEMS_PER_PRODUCER: usize = 250;
        const PRODUCERS: usize = 4;
        const TOTAL: usize = ITEMS_PER_PRODUCER * PRODUCERS;

        let q = Arc::new(MultiQueue::<usize>::with_threads(PRODUCERS));

        let mut handles = Vec::new();
        for t in 0..PRODUCERS {
            let q = Arc::clone(&q);
            handles.push(thread::spawn(move || {
                for i in 0..ITEMS_PER_PRODUCER {
                    q.push(t * ITEMS_PER_PRODUCER + i);
                }
            }));
        }

        let consumed = Arc::new(AtomicUsize::new(0));
        let q2 = Arc::clone(&q);
        let c = Arc::clone(&consumed);
        let consumer = thread::spawn(move || {
            while c.load(Relaxed) < TOTAL {
                // Best-of-2 fast path with exhaustive fallback.
                if q2.pop().or_else(|| q2.pop_any()).is_some() {
                    c.fetch_add(1, Relaxed);
                } else {
                    thread::yield_now();
                }
            }
        });

        for h in handles {
            h.join().unwrap();
        }
        consumer.join().unwrap();
        assert_eq!(consumed.load(Relaxed), TOTAL);
    }

    #[test]
    fn concurrent_multi_producer_multi_consumer_exactly_once() {
        const PRODUCERS: usize = 8;
        const CONSUMERS: usize = 8;
        const ITEMS_PER_PRODUCER: usize = 2_000;
        const TOTAL: usize = PRODUCERS * ITEMS_PER_PRODUCER;

        let q = Arc::new(MultiQueue::<usize>::with_threads(CONSUMERS));
        let start = Arc::new(Barrier::new(PRODUCERS + CONSUMERS));
        let producers_done = Arc::new(AtomicUsize::new(0));
        let consumed = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new((0..TOTAL).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());

        let mut handles = Vec::new();
        for producer in 0..PRODUCERS {
            let q = Arc::clone(&q);
            let start = Arc::clone(&start);
            let producers_done = Arc::clone(&producers_done);
            handles.push(thread::spawn(move || {
                start.wait();
                for offset in 0..ITEMS_PER_PRODUCER {
                    q.push(producer * ITEMS_PER_PRODUCER + offset);
                }
                producers_done.fetch_add(1, Relaxed);
            }));
        }

        for _ in 0..CONSUMERS {
            let q = Arc::clone(&q);
            let start = Arc::clone(&start);
            let producers_done = Arc::clone(&producers_done);
            let consumed = Arc::clone(&consumed);
            let seen = Arc::clone(&seen);
            handles.push(thread::spawn(move || {
                start.wait();
                loop {
                    if let Some(item) = q.pop().or_else(|| q.pop_any()) {
                        assert!(item < TOTAL, "out-of-range item {item}");
                        assert_eq!(
                            seen[item].fetch_add(1, Relaxed),
                            0,
                            "item {item} popped more than once"
                        );
                        consumed.fetch_add(1, Relaxed);
                    } else if producers_done.load(Relaxed) == PRODUCERS && q.is_empty() {
                        break;
                    } else {
                        thread::yield_now();
                    }
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(consumed.load(Relaxed), TOTAL);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(seen.iter().all(|count| count.load(Relaxed) == 1));
    }

    #[test]
    #[should_panic(expected = "at least 2 sub-queues")]
    fn panics_with_fewer_than_2_queues() {
        let _: MultiQueue<u32> = MultiQueue::new(1);
    }

    #[test]
    #[should_panic(expected = "at least 1")]
    fn with_threads_panics_on_zero() {
        let _: MultiQueue<u32> = MultiQueue::with_threads(0);
    }

    #[test]
    fn debug_shows_num_queues_and_len() {
        let q = MultiQueue::with_threads(3);
        q.push(1u32);
        let s = format!("{q:?}");
        assert!(s.contains("num_queues: 6"), "debug: {s}");
        assert!(s.contains("len: 1"), "debug: {s}");
    }
}
