use std::collections::{BinaryHeap, VecDeque};

use super::keyed::Keyed;
use crate::{default_policy, RunQueue, SchedFlavor, TaskView};

/// Oldest-request-first limited processor sharing (TailClipper §3.2/§4).
/// The window always contains the C oldest runnable tasks. Each dispatch is
/// one cooperative slice; a task that yields is appended to the window again.
/// This queue cannot preempt a future inside `poll`.
///
/// Smaller metadata values are older global arrival times. Infrastructure can
/// optionally use a separate FIFO lane. C must be positive.
pub struct BinaryHeapRoundRobinQueue<T, const USE_INFRA_QUEUE: bool = false, const C: usize = 6> {
    heap: BinaryHeap<Keyed<T>>,
    rr_queue: VecDeque<Keyed<T>>,
    infra_rr_queue: VecDeque<T>,
}

impl<T, const I: bool, const C: usize> BinaryHeapRoundRobinQueue<T, I, C> {
    fn fill(&mut self) {
        while self.rr_queue.len() < C {
            match self.heap.pop() {
                Some(item) => self.rr_queue.push_back(item),
                None => break,
            }
        }
    }
}

impl<T, const I: bool, const C: usize> RunQueue<T> for BinaryHeapRoundRobinQueue<T, I, C> {
    const FLAVOR: SchedFlavor = SchedFlavor::Prio;

    fn with_capacity(cap: usize) -> Self {
        assert!(C > 0, "TailClipper sharing window must be positive");
        Self {
            heap: BinaryHeap::with_capacity(cap),
            rr_queue: VecDeque::with_capacity(C),
            infra_rr_queue: VecDeque::new(),
        }
    }

    fn push(&mut self, item: T, view: &TaskView<'_>) {
        if I && default_policy::is_infra(view.meta) {
            self.infra_rr_queue.push_back(item);
            return;
        }
        // Include waiting candidates before comparing a new arrival. Do not
        // refill immediately after pop: the running task may yield back first.
        self.heap.push(Keyed::new(*view.meta, item));
        if self.rr_queue.is_empty() {
            return;
        }
        self.fill();
        while let Some(best) = self.heap.peek() {
            let worst = self
                .rr_queue
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.cmp(b.1))
                .map(|(i, _)| i);
            let Some(worst) = worst else { break };
            if best <= &self.rr_queue[worst] {
                break;
            }
            let replacement = self.heap.pop().expect("peeked candidate");
            let evicted = self.rr_queue.remove(worst).expect("window index");
            self.heap.push(evicted);
            self.rr_queue.push_back(replacement);
        }
    }

    fn pop(&mut self) -> Option<T> {
        if I {
            if let Some(item) = self.infra_rr_queue.pop_front() {
                return Some(item);
            }
        }
        self.fill();
        self.rr_queue.pop_front().map(|keyed| keyed.item)
    }

    fn len(&self) -> usize {
        self.heap.len() + self.rr_queue.len() + self.infra_rr_queue.len()
    }
    fn capacity(&self) -> usize {
        self.heap.capacity() + self.rr_queue.capacity() + self.infra_rr_queue.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Meta;

    const N: usize = 6;

    type Queue<const INFRA: bool = false> = BinaryHeapRoundRobinQueue<u64, INFRA>;

    /// Pushes task `id` with priority value `prio`.
    fn push<const INFRA: bool>(queue: &mut Queue<INFRA>, id: u64, prio: u64) {
        let t0 = std::time::Instant::now();
        queue.push(id, &TaskView::new(id, &Meta::new(prio), t0, t0, 0));
    }

    #[test]
    fn prio_bh_rr_queue_sched_flavor() {
        assert_eq!(<Queue as RunQueue<u64>>::FLAVOR, SchedFlavor::Prio);
    }

    #[test]
    fn infra_priority() {
        let mut queue = Queue::<true>::with_capacity(0);

        push(&mut queue, 1, 100);
        push(&mut queue, 2, 0);
        push(&mut queue, 3, 50);

        // Infra task should come out first
        assert_eq!(queue.pop(), Some(2));
        // Then the higher priority normal task (smaller prio value = higher priority)
        assert_eq!(queue.pop(), Some(3));
        // Then the last one
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn round_robin_refill() {
        let mut queue = Queue::<false>::with_capacity(0);

        // Push N+1 tasks. Higher priority (smaller val) comes out of heap first.
        for i in 0..N as u64 + 1 {
            push(&mut queue, i, (i + 1) * 10);
        }

        // 1st pop: RR empty. Refills from heap with the top N priorities.
        // The next N-1 pops come from that refill.
        for i in 0..N as u64 {
            assert_eq!(queue.pop(), Some(i));
        }

        // RR empty again: refills from the heap with the last item.
        assert_eq!(queue.pop(), Some(N as u64));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn arrival_fills_vacancy_without_evicting_an_active_task() {
        let mut queue = Queue::<false>::with_capacity(0);

        for i in 0..N as u64 {
            push(&mut queue, i, (i + 1) * 10);
        }

        // Pop one to refill RR with N items; pop returns T0, leaving N-1.
        assert_eq!(queue.pop(), Some(0));

        // There is a vacancy, so the older arrival joins the existing window.
        push(&mut queue, 100, 5);

        for i in 1..N as u64 {
            assert_eq!(queue.pop(), Some(i));
        }
        assert_eq!(queue.pop(), Some(100));

        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn push_no_eviction_when_priority_low() {
        let mut queue = Queue::<false>::with_capacity(0);

        for i in 0..N as u64 {
            push(&mut queue, i, (i + 1) * 10);
        }
        assert_eq!(queue.pop(), Some(0));

        // A task worse than anything in RR stays in the heap.
        push(&mut queue, 100, (N as u64 + 1) * 10);

        for i in 1..N as u64 {
            assert_eq!(queue.pop(), Some(i));
        }
        assert_eq!(queue.pop(), Some(100));
    }

    #[test]
    fn len_and_capacity() {
        let mut queue = Queue::<false>::with_capacity(0);
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());

        push(&mut queue, 1, 10);
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        push(&mut queue, 2, 0);
        assert_eq!(queue.len(), 2);

        assert!(queue.capacity() >= 2);

        queue.pop();
        queue.pop();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }
}

#[cfg(test)]
mod paper_tests {
    use super::*;
    use crate::Meta;
    use std::time::Instant;
    fn push<const C: usize>(q: &mut BinaryHeapRoundRobinQueue<u64, false, C>, id: u64) {
        let now = Instant::now();
        q.push(id, &TaskView::new(id, &Meta::new(id), now, now, 0));
    }
    #[test]
    fn only_two_oldest_share_until_one_blocks() {
        let mut q = BinaryHeapRoundRobinQueue::<u64, false, 2>::with_capacity(0);
        for id in 1..=4 {
            push(&mut q, id);
        }
        for round in 0..100 {
            let id = q.pop().unwrap();
            assert_eq!(id, round % 2 + 1);
            push(&mut q, id);
        }
        assert_eq!(q.pop(), Some(1)); // blocked: no re-enqueue
        assert_eq!(q.pop(), Some(2));
        push(&mut q, 2);
        assert_eq!(q.pop(), Some(3)); // vacancy filled without draining window
    }
    #[test]
    fn c_one_is_global_fcfs_even_when_oldest_yields() {
        let mut q = BinaryHeapRoundRobinQueue::<u64, false, 1>::with_capacity(0);
        for id in (1..=4).rev() {
            push(&mut q, id);
        }
        for _ in 0..20 {
            assert_eq!(q.pop(), Some(1));
            push(&mut q, 1);
        }
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
    }
    #[test]
    fn awakened_older_request_displaces_youngest_without_resetting_rotation() {
        let mut q = BinaryHeapRoundRobinQueue::<u64, false, 2>::with_capacity(0);
        for id in 2..=5 {
            push(&mut q, id);
        }
        assert_eq!(q.pop(), Some(2));
        push(&mut q, 2); // [3,2] active
        push(&mut q, 1); // oldest wakes, displaces 3
        assert_eq!(q.pop(), Some(2));
        push(&mut q, 2);
        assert_eq!(q.pop(), Some(1));
    }
    #[test]
    #[should_panic(expected = "sharing window must be positive")]
    fn zero_window_is_invalid() {
        let _ = BinaryHeapRoundRobinQueue::<u64, false, 0>::with_capacity(0);
    }
}
