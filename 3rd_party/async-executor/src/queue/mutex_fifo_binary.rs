use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use super::{PopError, PushError, Queue};
use tonic_masa::DeadlineHint;

pub(crate) struct MutexFifoBinaryQueue<T> {
    q_infra: Mutex<VecDeque<T>>,
    q_others: Mutex<VecDeque<T>>,
}

impl<T: Ord + PartialOrd> Queue for MutexFifoBinaryQueue<T> {
    type Item = T;

    fn push(&self, _item: Self::Item) -> Result<(), PushError<Self::Item>> {
        panic!("Not implemented for MutexFifoBinaryQueue");
    }

    fn push_with_ddl(
        &self,
        item: Self::Item,
        ddl: DeadlineHint,
    ) -> Result<(), PushError<Self::Item>> {
        if ddl.value() == 0 {
            self.with_locked_q_infra(|mut q| {
                q.push_back(item);
            });
        } else {
            self.with_locked_q_others(|mut q| {
                q.push_back(item);
            });
        }
        Ok(())
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        let infra_pop = self.with_locked_q_infra(|mut q| q.pop_front());
        match infra_pop {
            Some(item) => Ok(item),
            None => self
                .with_locked_q_others(|mut q| q.pop_front())
                .ok_or(PopError::Empty),
        }
    }

    fn len(&self) -> usize {
        self.with_locked_q_infra(|q| q.len()) + self.with_locked_q_others(|q| q.len())
    }

    fn is_full(&self) -> bool {
        // [NOTE] This implementation is unbounded so it is never full.
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(
            self.with_locked_q_infra(|q| q.capacity())
                + self.with_locked_q_others(|q| q.capacity()),
        )
    }
}

impl<T> MutexFifoBinaryQueue<T> {
    #[inline]
    fn with_locked_q_infra<R>(&self, f: impl FnOnce(MutexGuard<'_, VecDeque<T>>) -> R) -> R {
        let guard = self.q_infra.lock().expect("Mutex should not be poisoned");
        f(guard)
    }

    #[inline]
    fn with_locked_q_others<R>(&self, f: impl FnOnce(MutexGuard<'_, VecDeque<T>>) -> R) -> R {
        let guard = self.q_others.lock().expect("Mutex should not be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for MutexFifoBinaryQueue<T> {
    fn default() -> Self {
        Self {
            q_infra: Mutex::new(VecDeque::new()),
            q_others: Mutex::new(VecDeque::new()),
        }
    }
}
