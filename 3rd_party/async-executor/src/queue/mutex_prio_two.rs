use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

use super::{PopError, PushError, Queue};
use tonic_masa::{Prioritize, PriorityHint};

pub(crate) struct MutexPriorityTwoQueue<T> {
    q_active: Mutex<BinaryHeap<T>>,
    q_passive: Mutex<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd + Prioritize> Queue for MutexPriorityTwoQueue<T> {
    type Item = T;

    fn push(&self, _item: Self::Item) -> Result<(), PushError<Self::Item>> {
        panic!("Not implemented");
    }

    fn push_with_prio(
        &self,
        item: Self::Item,
        prio: PriorityHint,
    ) -> Result<(), PushError<Self::Item>> {
        // [TODO] Instead of passing `prio`, can we get it from `item`?
        // Such that, based on the value, we can push it to the active or passive queue.
        // If a value is before the current time, it should be pushed to the active queue.
        // Otherwise, it should be pushed to the passive queue.
        if prio.value() == 0 {
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
        // [TODO] When we pop an item, we should check if the item is after the current time.
        // If it is, we should push it to the passive queue.
        let infra_pop = self.with_locked_q_active(|mut q| q.pop());
        match infra_pop {
            Some(item) => Ok(item),
            None => self
                .with_locked_q_passive(|mut q| q.pop())
                .ok_or(PopError::Empty),
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
