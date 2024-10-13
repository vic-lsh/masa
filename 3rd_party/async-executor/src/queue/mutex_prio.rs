use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

use super::{PopError, PushError, Queue};
use tonic_masa::{Prioritize, PriorityHint};

pub(crate) struct MutexPriorityQueue<T> {
    q: Mutex<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd + Prioritize> Queue for MutexPriorityQueue<T> {
    type Item = T;

    fn push(&self, _item: Self::Item) -> Result<(), PushError<Self::Item>> {
        panic!("Not implemented");
    }

    fn push_with_prio(
        &self,
        item: Self::Item,
        _prio: PriorityHint,
    ) -> Result<(), PushError<Self::Item>> {
        self.with_locked(|mut q| {
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
