use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::collections::{BinaryHeap, VecDeque};
use std::cmp::Ordering;

// Mock dependencies to avoid dragging in the whole runtime

// Mock PriorityHint from masa_core
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PriorityHint(u64);

impl PriorityHint {
    pub fn value(&self) -> u64 {
        self.0
    }
}

pub trait Prioritize {
    fn priority(&self) -> PriorityHint;
}

// Mock Item
#[derive(Debug, Clone, Eq, PartialEq)]
struct MockItem {
    priority: u64,
    id: u64,
}

impl Ord for MockItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl PartialOrd for MockItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Prioritize for MockItem {
    fn priority(&self) -> PriorityHint {
        PriorityHint(self.priority)
    }
}

// Copied and modified BinaryHeapQueue from prio_bh.rs
// Removed Identifiable, Traceable constraints and timer calls for pure structure benchmarking

pub struct BinaryHeapQueue<T> {
    q: BinaryHeap<T>,
    infra_q: VecDeque<T>,
    push_count: u64,
}

impl<T: Ord + PartialOrd + Prioritize> BinaryHeapQueue<T> {
    fn new() -> Self {
        Self {
            q: BinaryHeap::new(),
            infra_q: VecDeque::new(),
            push_count: 0,
        }
    }

    fn push(&mut self, item: T) {
        // item.timer().set_enqueue_time(); // Removed
        
        if item.priority().value() == 0 {
            self.infra_q.push_back(item);
        } else {
            self.q.push(item);
        }
        
        self.push_count += 1;
    }

    fn pop(&mut self) -> Option<T> {
        if let Some(e) = self.infra_q.pop_front() {
            // e.timer().record_queue_lat(); // Removed
            return Some(e);
        }
        
        self.q.pop().map(|e| {
            // e.timer().record_queue_lat(); // Removed
            e
        })
    }
    
    fn len(&self) -> usize {
        self.q.len() + self.infra_q.len()
    }
}

// Old implementation (everything in BinaryHeap) for comparison
pub struct OldBinaryHeapQueue<T> {
    q: BinaryHeap<T>,
    push_count: u64,
}

impl<T: Ord + PartialOrd + Prioritize> OldBinaryHeapQueue<T> {
    fn new() -> Self {
        Self {
            q: BinaryHeap::new(),
            push_count: 0,
        }
    }

    fn push(&mut self, item: T) {
        self.q.push(item);
        self.push_count += 1;
    }

    fn pop(&mut self) -> Option<T> {
        self.q.pop()
    }
}

fn bench_queues(c: &mut Criterion) {
    let mut group = c.benchmark_group("PrioBHQueue");
    
    // Benchmark Infra Tasks (Priority 0)
    // The improvement should be visible here: O(1) vs O(log N)
    for size in [10, 30, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("Infra_New_O(1)", size), size, |b, &s| {
            let mut q = BinaryHeapQueue::new();
            // Pre-fill
            for i in 0..s {
                q.push(MockItem { priority: 0, id: i as u64 });
            }
            b.iter(|| {
                q.push(MockItem { priority: 0, id: 99999 });
                q.pop();
            })
        });

        group.bench_with_input(BenchmarkId::new("Infra_Old_O(logN)", size), size, |b, &s| {
            let mut q = OldBinaryHeapQueue::new();
            // Pre-fill
            for i in 0..s {
                q.push(MockItem { priority: 0, id: i as u64 });
            }
            b.iter(|| {
                q.push(MockItem { priority: 0, id: 99999 });
                q.pop();
            })
        });
    }

    // Benchmark Normal Tasks (Priority > 0)
    // Should be similar performance (both use BinaryHeap)
    for size in [10, 30, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("Normal_New", size), size, |b, &s| {
            let mut q = BinaryHeapQueue::new();
            for i in 0..s {
                q.push(MockItem { priority: (i % 100 + 1) as u64, id: i as u64 });
            }
            b.iter(|| {
                q.push(MockItem { priority: 50, id: 99999 });
                q.pop();
            })
        });

        group.bench_with_input(BenchmarkId::new("Normal_Old", size), size, |b, &s| {
            let mut q = OldBinaryHeapQueue::new();
            for i in 0..s {
                q.push(MockItem { priority: (i % 100 + 1) as u64, id: i as u64 });
            }
            b.iter(|| {
                q.push(MockItem { priority: 50, id: 99999 });
                q.pop();
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_queues);
criterion_main!(benches);
