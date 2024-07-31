use log::info;

use super::{PopError, PushError, Queue};
use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

pub(crate) struct MutexPriorityQueue<T> {
    q: Mutex<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd> Queue for MutexPriorityQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.with_locked(|mut q| {
            // [DEBUG] to determine whether requests are reordered by deadline
            let smaller_elems_cnt = q.iter().filter(|&e| e < &item).count();
            let larger_elems_cnt = q.len() - smaller_elems_cnt;
            info!(
                "Pushing into PQueue: {} before, {} after",
                smaller_elems_cnt, larger_elems_cnt
            );

            q.push(item);
        });
        Ok(())
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        self.with_locked(|mut q| q.pop()).ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.with_locked(|q| q.len())
    }

    fn is_full(&self) -> bool {
        // [NOTE] this implementation is unbounded so it's never full
        false
    }

    fn capacity(&self) -> Option<usize> {
        self.with_locked(|q| Some(q.capacity()))
    }
}

impl<T> MutexPriorityQueue<T> {
    #[inline]
    fn with_locked<R>(&self, f: impl FnOnce(MutexGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q.lock().expect("mutex shouldn't be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for MutexPriorityQueue<T> {
    fn default() -> Self {
        Self {
            q: Mutex::new(BinaryHeap::new()),
        }
    }
}
