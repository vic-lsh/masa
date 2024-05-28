use std::{
    collections::BinaryHeap,
    sync::{Mutex, MutexGuard},
};

use concurrent_queue::ConcurrentQueue;

#[allow(dead_code)]
pub(crate) trait Queue {
    type Item;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>>;

    fn pop(&self) -> Result<Self::Item, PopError>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_full(&self) -> bool;

    /// Refer to implementations for when Some(_) or None is returned.
    fn capacity(&self) -> Option<usize>;
}

#[derive(Debug)]
pub(crate) enum PushError<T> {
    Full(T),
    Closed(T),
}

#[derive(Debug)]
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

    fn capacity(&self) -> Option<usize> {
        self.with_locked(|q| Some(q.capacity()))
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

pub(crate) struct ConcurrentFifoQueue<T> {
    q: ConcurrentQueue<T>,
}

impl<T> Default for ConcurrentFifoQueue<T> {
    fn default() -> Self {
        Self::bounded(512)
    }
}

impl<T> ConcurrentFifoQueue<T> {
    pub(crate) fn bounded(size: usize) -> Self {
        Self {
            q: ConcurrentQueue::bounded(size),
        }
    }
}

impl<T> Queue for ConcurrentFifoQueue<T> {
    type Item = T;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.q.push(item).map_err(|e| match e {
            concurrent_queue::PushError::Full(item) => PushError::Full(item),
            concurrent_queue::PushError::Closed(item) => PushError::Closed(item),
        })
    }

    fn pop(&self) -> Result<Self::Item, PopError> {
        self.q.pop().map_err(|e| match e {
            concurrent_queue::PopError::Empty => PopError::Empty,
            concurrent_queue::PopError::Closed => PopError::Closed,
        })
    }

    fn len(&self) -> usize {
        self.q.len()
    }

    fn is_full(&self) -> bool {
        self.q.is_full()
    }

    fn capacity(&self) -> Option<usize> {
        self.q.capacity()
    }
}
