use concurrent_queue::ConcurrentQueue;

use super::{PopError, PushError, Queue};

pub(crate) struct ConcurrentFifoQueue<T> {
    q: ConcurrentQueue<T>,
}

impl<T> Default for ConcurrentFifoQueue<T> {
    fn default() -> Self {
        Self::unbounded()
    }
}

impl<T> ConcurrentFifoQueue<T> {
    pub(crate) fn unbounded() -> Self {
        Self {
            q: ConcurrentQueue::unbounded(),
        }
    }

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
