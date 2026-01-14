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

const N: usize = 6;

pub(crate) struct BinaryHeapRoundRobinQueue<T, const USE_INFRA_QUEUE: bool = false> {
    heap: BinaryHeap<T>,
    rr_queue: VecDeque<T>,
    infra_rr_queue: VecDeque<T>,
}

impl<T: Ord + PartialOrd + Prioritize + Identifiable, const USE_INFRA_QUEUE: bool> Queue
    for BinaryHeapRoundRobinQueue<T, USE_INFRA_QUEUE>
{
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(cap),
            rr_queue: VecDeque::with_capacity(N),
            infra_rr_queue: VecDeque::with_capacity(cap),
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        if USE_INFRA_QUEUE && item.priority() == PriorityHint::infra() {
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
        if USE_INFRA_QUEUE {
            if let Some(item) = self.infra_rr_queue.pop_front() {
                return Ok(item);
            }
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

impl<T: Ord, const USE_INFRA_QUEUE: bool> Default
    for BinaryHeapRoundRobinQueue<T, USE_INFRA_QUEUE>
{
    fn default() -> Self {
        Self {
            heap: BinaryHeap::new(),
            rr_queue: VecDeque::new(),
            infra_rr_queue: VecDeque::new(),
        }
    }
}

impl<T, const USE_INFRA_QUEUE: bool> IntoSchedFlavor
    for BinaryHeapRoundRobinQueue<T, USE_INFRA_QUEUE>
{
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
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask, true>::default();

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

        // Push N+1 tasks.
        // Higher priority (smaller val) comes out of heap first.
        for i in 0..N + 1 {
            queue
                .push(MockTask::new(i as u64, (i + 1) as u64 * 10))
                .unwrap();
        }

        // 1st pop: RR empty. Refills from heap.
        // Heap should yield top N priorities.
        // Pop returns first item.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 0);

        // Next N-1 pops should come from the initial RR refill.
        for i in 1..N {
            let popped = queue.pop().unwrap();
            assert_eq!(popped.id.0, i as u64);
        }

        // Next pop: RR empty. Refills from heap with the last item.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, N as u64);

        assert!(queue.pop().is_err());
    }

    #[test]
    fn test_eviction_logic() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();

        // Step 1: Populate queue and trigger a refill to move items to RR.
        // Push N tasks.
        for i in 0..N {
            queue
                .push(MockTask::new(i as u64, (i + 1) as u64 * 10))
                .unwrap();
        }

        // Pop one to trigger refill.
        // RR gets N items. Pop returns T0.
        // RR now has N-1 items.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 0);

        // Step 2: Push a high priority task T_new(5).
        // It should evict the lowest priority item currently in RR.
        // The lowest priority item in RR is the one with the largest priority value.
        // That is T_{N-1} with priority N*10.
        queue.push(MockTask::new(100, 5)).unwrap();

        // The RR queue originally had [T1, T2, ..., T_{N-1}].
        // After eviction and push, it should have [T1, T2, ..., T_{N-2}, T_new].
        // T_new is pushed to the back of the VecDeque.

        // Pop all from RR.
        for i in 1..N - 1 {
            let popped = queue.pop().unwrap();
            assert_eq!(popped.id.0, i as u64);
        }

        // Next is T_new.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 100);

        // Finally, the evicted T_{N-1} from heap.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, (N - 1) as u64);
    }

    #[test]
    fn test_push_no_eviction_when_priority_low() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();

        // Push N tasks.
        for i in 0..N {
            queue
                .push(MockTask::new(i as u64, (i + 1) as u64 * 10))
                .unwrap();
        }

        // Pop 1 to refill RR.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 0);

        // RR has [T1, ..., T_{N-1}].
        // Push T_low with priority higher value than any in RR.
        let low_prio_val = (N + 1) as u64 * 10;
        queue.push(MockTask::new(100, low_prio_val)).unwrap();

        // All items currently in RR should be popped first.
        for i in 1..N {
            let popped = queue.pop().unwrap();
            assert_eq!(popped.id.0, i as u64);
        }

        // Then T_low from heap.
        let popped = queue.pop().unwrap();
        assert_eq!(popped.id.0, 100);
    }

    #[test]
    fn test_len_and_capacity() {
        let mut queue = BinaryHeapRoundRobinQueue::<MockTask>::default();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
        assert!(!queue.is_full());

        queue.push(MockTask::new(1, 10)).unwrap();
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.push(MockTask::new_infra(2)).unwrap();
        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());

        // Capacity check
        assert!(queue.capacity().unwrap() >= 2);
        assert!(!queue.is_full());

        queue.pop().unwrap();
        queue.pop().unwrap();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }
}
