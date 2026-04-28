use std::collections::{BinaryHeap, VecDeque};
use std::sync::Mutex;

use crate::runtime::task::{Identifiable, Traceable};
use masa_core::Prioritize;

/// Thread-safe priority queue for the multi-thread scheduler under `sched_mt`.
///
/// A single shared heap protected by one mutex. All worker threads push and pop
/// through this queue, which guarantees global priority ordering (EDF) at the
/// cost of lock contention. Infrastructure tasks (PriorityHint == 0) are kept
/// in a separate VecDeque and always drain before user tasks.
pub(crate) struct SharedPrioQueue<T> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T> {
    q: BinaryHeap<T>,
    infra_q: VecDeque<T>,
}

impl<T: Ord + Prioritize + Identifiable + Traceable> SharedPrioQueue<T> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                q: BinaryHeap::new(),
                infra_q: VecDeque::new(),
            }),
        }
    }

    pub(crate) fn push(&self, mut item: T) {
        item.timer().set_enqueue_time();
        let mut inner = self.inner.lock().unwrap();
        if item.priority().value() == 0 {
            inner.infra_q.push_back(item);
        } else {
            inner.q.push(item);
        }
    }

    pub(crate) fn pop(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut e) = inner.infra_q.pop_front() {
            e.timer().record_queue_lat();
            return Some(e);
        }
        inner.q.pop().map(|mut e| {
            e.timer().record_queue_lat();
            e
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.q.is_empty() && inner.infra_q.is_empty()
    }

}
