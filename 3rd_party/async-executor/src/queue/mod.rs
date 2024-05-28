use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

pub(crate) trait Queue {
    type Item;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>>;

    fn pop(&self) -> Result<Self::Item, PopError>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_full(&self) -> bool;
}

pub(crate) enum PushError<T> {
    Full(T),
    Closed,
}

pub(crate) enum PopError {
    Empty,
    Closed,
}

pub(crate) struct SimplePriorityQueue<T> {
    q: Mutex<BinaryHeap<T>>,
}

impl<T: Ord + PartialOrd> Queue for SimplePriorityQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.with_locked(|mut q| q.push(item));
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
}

impl<T> SimplePriorityQueue<T> {
    #[inline]
    fn with_locked<R>(&self, f: impl FnOnce(MutexGuard<'_, BinaryHeap<T>>) -> R) -> R {
        let guard = self.q.lock().expect("mutex shouldn't be poisoned");
        f(guard)
    }
}

impl<T: Ord> Default for SimplePriorityQueue<T> {
    fn default() -> Self {
        Self {
            q: Mutex::new(BinaryHeap::new()),
        }
    }
}
