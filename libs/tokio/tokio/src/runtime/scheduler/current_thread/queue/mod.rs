mod fifo;
mod prio_bh;
mod timed;

#[cfg(feature = "scheduler_trace")]
pub(crate) type LocalRunQueue<T> = timed::TimedQueue<LocalRunQueueInner<T>>;
#[cfg(not(feature = "scheduler_trace"))]
pub(crate) type LocalRunQueue<T> = LocalRunQueueInner<T>;

#[cfg(not(any(
    feature = "prio_class",
    feature = "prio_global",
    feature = "prio_global_early",
    feature = "prio_class_global",
    feature = "prio_local",
    feature = "prio_local_early",
    feature = "perfect_lsf",
    feature = "fifo_infra",
    feature = "fifo"
)))]
pub(crate) type LocalRunQueueInner<T> = fifo::FifoQueue<T>;

#[cfg(any(feature = "fifo", feature = "fifo_infra"))]
pub(crate) type LocalRunQueueInner<T> = fifo::FifoQueue<T>;

#[cfg(any(
    feature = "prio_global",
    feature = "prio_local",
    feature = "prio_global_early",
    feature = "prio_local_early",
    feature = "perfect_lsf"
))]
pub(crate) type LocalRunQueueInner<T> = prio_bh::BinaryHeapQueue<T>;

/// Describes the different strategies implemented by Masa.
#[derive(PartialEq, Eq, Debug)]
pub enum SchedFlavor {
    /// The scheduler executes tasks in first-in-first-out order.
    Fifo,
    /// The scheduler executes tasks based on priority.
    Prio,
}

trait IntoSchedFlavor {
    // Note that this does not operate on a concrete struct instance. It operates
    // on the struct type information only.
    fn into_sched_flavor() -> SchedFlavor;
}

/// Get the scheduling flavor used by this runtime instantiation.
pub fn get_sched_flavor() -> SchedFlavor {
    LocalRunQueueInner::<u64>::into_sched_flavor()
}

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

    fn with_capacity(cap: usize) -> Self;
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum PushError<T> {
    Full(T),
    Closed(T),
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum PopError {
    Empty,
    Closed,
}
