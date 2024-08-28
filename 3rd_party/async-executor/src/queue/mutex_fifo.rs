use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use super::{PopError, PushError, Queue};
use tonic_masa::DeadlineHint;

pub(crate) struct MutexFifoQueue<T> {
    q: Mutex<VecDeque<T>>,
}

impl<T: Ord + PartialOrd> Queue for MutexFifoQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.with_locked(|mut q| {
            q.push_back(item);
        });
        Ok(())
    }

    fn push_with_ddl(
        &self,
        item: Self::Item,
        _ddl: DeadlineHint,
    ) -> Result<(), PushError<Self::Item>> {
        self.push(item)
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        self.with_locked(|mut q| q.pop_front())
            .ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.with_locked(|q| q.len())
    }

    fn is_full(&self) -> bool {
        // [NOTE] This implementation is unbounded so it is never full.
        false
    }

    fn capacity(&self) -> Option<usize> {
        self.with_locked(|q| Some(q.capacity()))
    }
}

impl<T> MutexFifoQueue<T> {
    #[inline]
    fn with_locked<R>(&self, f: impl FnOnce(MutexGuard<'_, VecDeque<T>>) -> R) -> R {
        let guard = self.q.lock().expect("mutex shouldn't be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for MutexFifoQueue<T> {
    fn default() -> Self {
        Self {
            q: Mutex::new(VecDeque::new()),
        }
    }
}
