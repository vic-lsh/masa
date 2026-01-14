use std::{
    collections::{BinaryHeap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{IntoSchedFlavor, PopError, PushError, Queue, SchedFlavor};
use crate::runtime::task::Identifiable;
use masa::{Prioritize, PriorityHint};

#[allow(dead_code)]
static INIT: std::sync::LazyLock<u64> = std::sync::LazyLock::new(time_now);

#[inline]
fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[allow(dead_code)]
fn ms_since_init(value: u64) -> u64 {
    (value - *INIT) / 1000
}

const N: usize = 3;

pub(crate) struct BinaryHeapRoundRobinQueue<T> {
    heap: BinaryHeap<T>,
    rr_queue: VecDeque<T>,
    infra_rr_queue: VecDeque<T>,
}

impl<T: Ord + PartialOrd + Prioritize + Identifiable> Queue for BinaryHeapRoundRobinQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(cap),
            rr_queue: VecDeque::with_capacity(N),
            infra_rr_queue: VecDeque::with_capacity(cap),
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        if item.priority() == PriorityHint::infra() {
            self.infra_rr_queue.push_back(item);
            return Ok(());
        }

        if !self.rr_queue.is_empty() {
            // Find index of item with minimum priority in rr_queue
            let mut min_idx = 0;
            for i in 1..self.rr_queue.len() {
                if self.rr_queue[i] < self.rr_queue[min_idx] {
                    min_idx = i;
                }
            }

            // If the new item has higher priority than the lowest-priority item in rr_queue, replace it.
            if item > self.rr_queue[min_idx] {
                let evicted = self.rr_queue.remove(min_idx).unwrap();
                self.heap.push(evicted);
                self.rr_queue.push_back(item);
                return Ok(());
            }
        }

        self.heap.push(item);
        Ok(())
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        if let Some(item) = self.infra_rr_queue.pop_front() {
            return Ok(item);
        }

        if self.rr_queue.is_empty() {
            // Refill the round-robin queue with the top N items from the heap
            for _ in 0..N {
                if let Some(item) = self.heap.pop() {
                    self.rr_queue.push_back(item);
                }
            }
        }

        self.rr_queue.pop_front().ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.heap.len() + self.rr_queue.len() + self.infra_rr_queue.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.heap.capacity() + self.rr_queue.capacity() + self.infra_rr_queue.capacity())
    }
}

impl<T: Ord> Default for BinaryHeapRoundRobinQueue<T> {
    fn default() -> Self {
        Self {
            heap: BinaryHeap::new(),
            rr_queue: VecDeque::new(),
            infra_rr_queue: VecDeque::new(),
        }
    }
}

impl<T> IntoSchedFlavor for BinaryHeapRoundRobinQueue<T> {
    fn into_sched_flavor() -> SchedFlavor {
        SchedFlavor::Prio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::task::Id;

    #[derive(Debug, Clone)]
    struct MockTask {
        id: Id,
        priority: PriorityHint,
    }

    impl MockTask {
        fn new(id: u64, priority: u64) -> Self {
            Self {
                id: Id(id),
                priority: PriorityHint::new(priority),
            }
        }
        fn new_infra(id: u64) -> Self {
            Self {
                id: Id(id),
                priority: PriorityHint::infra(),
            }
        }
    }

    impl PartialEq for MockTask {
        fn eq(&self, other: &Self) -> bool {
            self.priority == other.priority
        }
    }

    impl Eq for MockTask {}

    impl PartialOrd for MockTask {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for MockTask {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.priority.cmp(&other.priority)
        }
    }

    impl Prioritize for MockTask {
        fn priority(&self) -> PriorityHint {
            self.priority
        }
    }

    impl Identifiable for MockTask {
        fn id(&self) -> Id {
            self.id
        }
    }

    #[test]
    fn test_prio_bh_rr_queue_sched_flavor() {
        assert_eq!(
            BinaryHeapRoundRobinQueue::<u64>::into_sched_flavor(),
            SchedFlavor::Prio
        );
    }

    #[test]
    fn test_infra_priority() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();

        let t1 = MockTask::new(1, 100);
        let t2 = MockTask::new_infra(2);
        let t3 = MockTask::new(3, 50);

        queue.push(t1).unwrap();
        queue.push(t2).unwrap();
        queue.push(t3).unwrap();

        // Infra task should come out first
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 2);

        // Then the higher priority normal task (smaller prio value = higher priority)
        // t3 (50) > t1 (100)
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 3);

        // Then the last one
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 1);
    }

    #[test]
    fn test_round_robin_refill() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();

        // Push 4 tasks. N=3.
        // We use different priorities to make heap order deterministic for the test,
        // but close enough to illustrate the batching.
        // Actually, let's strictly control priorities to ensure heap order.
        // T1 (10), T2 (20), T3 (30), T4 (40).
        // Higher priority (smaller val) comes out of heap first.
        let t1 = MockTask::new(1, 10);
        let t2 = MockTask::new(2, 20);
        let t3 = MockTask::new(3, 30);
        let t4 = MockTask::new(4, 40);

        queue.push(t1).unwrap();
        queue.push(t2).unwrap();
        queue.push(t3).unwrap();
        queue.push(t4).unwrap();

        // Queue state: Heap has [T1, T2, T3, T4], RR is empty.

        // 1st pop: RR empty. Refills from heap.
        // Heap should yield T1, T2, T3 (top 3 priorities).
        // RR becomes [T1, T2, T3]. Heap has [T4].
        // Pop returns T1. RR is [T2, T3].
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 1);

        // 2nd pop: RR has items. Returns T2.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 2);

        // 3rd pop: RR has items. Returns T3.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 3);

        // 4th pop: RR empty. Refills from heap.
        // Heap yields T4.
        // RR becomes [T4].
        // Pop returns T4.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 4);

        assert!(queue.pop().is_err());
    }

    #[test]
    fn test_eviction_logic() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();

        // Step 1: Populate queue and trigger a refill to move items to RR.
        // Push T1(10), T2(20), T3(30).
        queue.push(MockTask::new(1, 10)).unwrap();
        queue.push(MockTask::new(2, 20)).unwrap();
        queue.push(MockTask::new(3, 30)).unwrap();

        // Pop one to trigger refill.
        // RR gets [T1, T2, T3].
        // Pop returns T1.
        // RR is now [T2, T3].
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 1);

        // Step 2: Push a high priority task T4(5).
        // RR has [T2(20), T3(30)].
        // Lowest priority in RR is T3(30) (remember higher value = lower priority).
        // T4(5) has higher priority than T3(30).
        // T3 should be evicted to heap. T4 added to RR.
        // RR: [T2(20), T4(5)]. Heap: [T3(30)].
        queue.push(MockTask::new(4, 5)).unwrap();

        // Verify next pop is from RR.
        // It's a VecDeque, so it pops from front.
        // T2 was at front. T4 was pushed back.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 2);

        // Next pop is T4.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 4);

        // RR is empty. Refill from heap.
        // Heap has T3.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 3);
    }
}
