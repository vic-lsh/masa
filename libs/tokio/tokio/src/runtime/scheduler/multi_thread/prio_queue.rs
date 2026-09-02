use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::runtime::task::{Identifiable, Traceable};
use crate::task::TaskPrioritize;

#[cfg(feature = "sched_mt_multiqueue")]
type MainQueue<T> = multiqueue_prio_queue::MultiQueue<T>;

#[cfg(not(feature = "sched_mt_multiqueue"))]
type MainQueue<T> = backend::MutexBackend<T>;

/// Thread-safe priority queue for the multi-thread scheduler under `sched_mt`.
///
/// Infrastructure tasks are kept in a separate FIFO and drained before user
/// tasks, regardless of the selected user-task backend.
pub(crate) struct SharedPrioQueue<T: Ord> {
    main: MainQueue<T>,
    infra: Mutex<VecDeque<T>>,
    infra_pending: AtomicBool,
}

impl<T: Ord + TaskPrioritize + Identifiable + Traceable + Send + 'static> SharedPrioQueue<T> {
    pub(crate) fn new(num_workers: usize) -> Self {
        #[cfg(not(feature = "sched_mt_multiqueue"))]
        let _ = num_workers;

        #[cfg(feature = "sched_mt_multiqueue")]
        let main = multiqueue_prio_queue::MultiQueue::with_threads(num_workers.max(1));

        #[cfg(not(feature = "sched_mt_multiqueue"))]
        let main = backend::MutexBackend::new();

        Self {
            main,
            infra: Mutex::new(VecDeque::new()),
            infra_pending: AtomicBool::new(false),
        }
    }

    pub(crate) fn push(&self, mut item: T) {
        item.timer().set_enqueue_time();
        if item.priority().value() == 0 {
            let mut infra = self.infra.lock().unwrap();
            infra.push_back(item);
            self.infra_pending.store(true, Ordering::Release);
        } else {
            self.main.push(item);
        }
    }

    pub(crate) fn pop(&self) -> Option<T> {
        // Normal user-task pops only read this flag. The shared FIFO lock is
        // touched only while infrastructure work is actually pending.
        if self.infra_pending.load(Ordering::Acquire) {
            let mut infra = self.infra.lock().unwrap();
            let item = infra.pop_front();
            if infra.is_empty() {
                self.infra_pending.store(false, Ordering::Release);
            }
            if let Some(mut item) = item {
                item.timer().record_queue_lat();
                return Some(item);
            }
        }

        #[cfg(feature = "sched_mt_multiqueue")]
        let item = self.main.pop().or_else(|| self.main.pop_any());

        #[cfg(not(feature = "sched_mt_multiqueue"))]
        let item = self.main.pop();

        item.map(|mut e| {
            e.timer().record_queue_lat();
            e
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.main.is_empty() && !self.infra_pending.load(Ordering::Acquire)
    }
}

#[cfg(not(feature = "sched_mt_multiqueue"))]
mod backend {
    use std::collections::BinaryHeap;
    use std::sync::Mutex;

    pub(super) struct MutexBackend<T> {
        inner: Mutex<BinaryHeap<T>>,
    }

    impl<T: Ord + Send> MutexBackend<T> {
        pub(super) fn new() -> Self {
            Self {
                inner: Mutex::new(BinaryHeap::new()),
            }
        }

        pub(super) fn push(&self, item: T) {
            self.inner.lock().unwrap().push(item);
        }

        pub(super) fn pop(&self) -> Option<T> {
            self.inner.lock().unwrap().pop()
        }

        pub(super) fn is_empty(&self) -> bool {
            self.inner.lock().unwrap().is_empty()
        }
    }
}
