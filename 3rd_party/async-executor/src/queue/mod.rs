mod concurrent_fifo;
mod mutex_pqueue;
mod rw_pqueue;

pub(crate) use concurrent_fifo::ConcurrentFifoQueue;
pub(crate) use mutex_pqueue::MutexPriorityQueue;

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
