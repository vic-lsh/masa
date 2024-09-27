use std::{
    collections::BinaryHeap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use super::{PopError, PushError, Queue};
use tonic_masa::PriorityHint;

#[allow(dead_code)]
pub(crate) struct RwPriorityQueue<T> {
    q: RwLock<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd> Queue for RwPriorityQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.with_write_lock(|mut q| q.push(item));
        Ok(())
    }

    fn push_with_ddl(
        &self,
        item: Self::Item,
        _ddl: PriorityHint,
    ) -> Result<(), PushError<Self::Item>> {
        self.push(item)
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        self.with_write_lock(|mut q| q.pop()).ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.with_read_lock(|q| q.len())
    }

    fn is_full(&self) -> bool {
        // [NOTE] this implementation is unbounded so it's never full
        false
    }

    fn capacity(&self) -> Option<usize> {
        self.with_read_lock(|q| Some(q.capacity()))
    }
}

#[allow(dead_code)]
impl<T> RwPriorityQueue<T> {
    #[inline]
    fn with_read_lock<R>(&self, f: impl FnOnce(RwLockReadGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q.read().expect("lock shouldn't be poisoned");
        f(guard)
    }

    #[inline]
    fn with_write_lock<R>(&self, f: impl FnOnce(RwLockWriteGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q.write().expect("lock shouldn't be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for RwPriorityQueue<T> {
    fn default() -> Self {
        Self {
            q: RwLock::new(BinaryHeap::new()),
        }
    }
}
