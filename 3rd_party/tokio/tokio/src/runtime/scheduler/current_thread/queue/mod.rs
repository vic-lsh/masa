mod fifo;
mod prio_bh;

#[cfg(not(any(
    feature = "prio_class",
    feature = "prio_global",
    feature = "prio_global_early",
    feature = "prio_class_global",
    feature = "prio_local",
    feature = "prio_local_early",
    feature = "fifo_infra",
    feature = "fifo"
)))]
pub(crate) type LocalRunQueue<T> = fifo::FifoQueue<T>;

#[cfg(feature = "fifo")]
pub(crate) type LocalRunQueue<T> = fifo::FifoQueue<T>;
#[cfg(any(
    feature = "prio_global",
    feature = "prio_local",
    feature = "prio_global_early",
    feature = "prio_local_early"
))]
pub(crate) type LocalRunQueue<T> = prio_bh::BinaryHeapQueue<T>;

#[allow(dead_code)]
pub(crate) trait Queue {
    type Item;

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>>;

    fn pop(&mut self) -> Result<Self::Item, PopError>;

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
