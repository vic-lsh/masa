mod concurrent_fifo;
mod mutex_fifo;
mod mutex_fifo_two;
mod mutex_prio;
mod rw_prio;

#[allow(dead_code)]
pub(crate) use concurrent_fifo::ConcurrentFifoQueue;
#[allow(dead_code)]
pub(crate) use mutex_fifo::MutexFifoQueue;
#[allow(dead_code)]
pub(crate) use mutex_fifo_two::MutexFifoTwoQueue;
#[allow(dead_code)]
#[allow(unused_imports)]
pub(crate) use mutex_prio::MutexPriorityQueue;

use tonic_masa::PriorityHint;

#[allow(dead_code)]
pub(crate) trait Queue {
    type Item;

    fn push(&self, item: Self::Item) -> Result<(), PushError<Self::Item>>;

    fn push_with_prio(
        &self,
        item: Self::Item,
        prio: PriorityHint,
    ) -> Result<(), PushError<Self::Item>>;

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
