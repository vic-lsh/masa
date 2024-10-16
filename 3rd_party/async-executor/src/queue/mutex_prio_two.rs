use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

use super::{time_now, PopError, PushError, Queue};
use tonic_masa::Prioritize;

pub(crate) struct MutexPriorityTwoQueue<T> {
    q_active: Mutex<BinaryHeap<T>>,
    q_passive: Mutex<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd + Prioritize> Queue for MutexPriorityTwoQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        let prio = item.priority();
        if time_now() < prio.value() {
            self.with_locked_q_active(|mut q| {
                q.push(item);
            });
        } else {
            self.with_locked_q_passive(|mut q| {
                q.push(item);
            });
        }
        Ok(())
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        // [NOTE] Pop from active queue first, check if it is still active,
        // if not, push to passive queue. If no valid item in active queue,
        // pop from passive queue.
        loop {
            let infra_pop = self.with_locked_q_active(|mut q| q.pop());
            match infra_pop {
                Some(item) => {
                    let prio = item.priority();
                    if time_now() < prio.value() {
                        return Ok(item);
                    } else {
                        self.with_locked_q_passive(|mut q| q.push(item));
                    }
                }
                None => {
                    if let Some(item) = self.with_locked_q_passive(|mut q| q.pop()) {
                        return Ok(item);
                    } else {
                        return Err(PopError::Empty);
                    }
                }
            }
        }
    }

    fn len(&self) -> usize {
        self.with_locked_q_active(|q| q.len()) + self.with_locked_q_passive(|q| q.len())
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(
            self.with_locked_q_active(|q| q.capacity())
                + self.with_locked_q_passive(|q| q.capacity()),
        )
    }
}

impl<T> MutexPriorityTwoQueue<T> {
    #[inline]
    fn with_locked_q_active<R>(&self, f: impl FnOnce(MutexGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q_active.lock().expect("Mutex should not be poisoned");
        f(guard)
    }

    #[inline]
    fn with_locked_q_passive<R>(&self, f: impl FnOnce(MutexGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q_passive.lock().expect("Mutex should not be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for MutexPriorityTwoQueue<T> {
    fn default() -> Self {
        Self {
            q_active: Mutex::new(BinaryHeap::new()),
            q_passive: Mutex::new(BinaryHeap::new()),
        }
    }
}
