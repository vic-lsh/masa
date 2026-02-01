pub(crate) mod fifo;
pub(crate) mod fifo_infra;
pub(crate) mod prio_bh;
pub(crate) mod prio_bh_rr;
pub(crate) mod timed;

pub(crate) type LocalRunQueue<T> = LocalRunQueueInner<T>;

pub(crate) enum LocalRunQueueInner<T> {
    Fifo(fifo::FifoQueue<T>),
    Prio(prio_bh::BinaryHeapQueue<T>),
    PrioOldest(prio_bh_rr::BinaryHeapRoundRobinQueue<T, false>),
}

impl<T> Queue for LocalRunQueueInner<T>
where
    T: Ord + PartialOrd + masa::Prioritize + crate::runtime::task::Identifiable,
{
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        // Default to Fifo if not specified
        Self::Fifo(fifo::FifoQueue::with_capacity(cap))
    }

    fn new(ty: Option<masa::QueueType>, cap: usize) -> Self {
        match ty {
            Some(masa::QueueType::Fifo) => Self::Fifo(fifo::FifoQueue::with_capacity(cap)),
            Some(masa::QueueType::Prio) => Self::Prio(prio_bh::BinaryHeapQueue::with_capacity(cap)),
            Some(masa::QueueType::PrioOldest) => Self::PrioOldest(prio_bh_rr::BinaryHeapRoundRobinQueue::with_capacity(cap)),
            None => Self::Fifo(fifo::FifoQueue::with_capacity(cap)),
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        match self {
            Self::Fifo(q) => q.push(item),
            Self::Prio(q) => q.push(item),
            Self::PrioOldest(q) => q.push(item),
        }
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        match self {
            Self::Fifo(q) => q.pop(),
            Self::Prio(q) => q.pop(),
            Self::PrioOldest(q) => q.pop(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Fifo(q) => q.len(),
            Self::Prio(q) => q.len(),
            Self::PrioOldest(q) => q.len(),
        }
    }

    fn is_full(&self) -> bool {
        match self {
            Self::Fifo(q) => q.is_full(),
            Self::Prio(q) => q.is_full(),
            Self::PrioOldest(q) => q.is_full(),
        }
    }

    fn capacity(&self) -> Option<usize> {
        match self {
            Self::Fifo(q) => q.capacity(),
            Self::Prio(q) => q.capacity(),
            Self::PrioOldest(q) => q.capacity(),
        }
    }

    fn sched_flavor(&self) -> SchedFlavor {
        match self {
            Self::Fifo(_) => SchedFlavor::Fifo,
            Self::Prio(_) => SchedFlavor::Prio,
            Self::PrioOldest(_) => SchedFlavor::Prio,
        }
    }
}

/// Describes the different strategies implemented by Masa.
#[derive(PartialEq, Eq, Debug)]
pub enum SchedFlavor {
    /// The scheduler executes tasks in first-in-first-out order.
    Fifo,
    /// The scheduler executes tasks based on priority.
    Prio,
}

/// Get the scheduling flavor used by this runtime instantiation.
pub fn get_sched_flavor() -> SchedFlavor {
    use crate::runtime::context;
    use crate::runtime::scheduler::Context;

    context::with_scheduler(|maybe_context| {
        let context = match maybe_context {
            Some(Context::CurrentThread(ctx)) => ctx,
            #[cfg(feature = "rt-multi-thread")]
            Some(_) => return SchedFlavor::Prio,
            None => return SchedFlavor::Fifo, // Default/Fallback
        };

        let core = context.core.borrow();
        match core.as_ref() {
            Some(core) => core.tasks.sched_flavor(),
            None => SchedFlavor::Fifo,
        }
    })
}

/// Get the current queue length for the current_thread runtime.
pub fn current_thread_queue_len() -> usize {
    use crate::runtime::context;
    use crate::runtime::scheduler::Context;

    context::with_scheduler(|maybe_context| {
        let context = match maybe_context {
            Some(Context::CurrentThread(ctx)) => ctx,
            #[cfg(feature = "rt-multi-thread")]
            Some(_) => panic!(
                "current_thread_queue_len() can only be called from a current_thread runtime"
            ),
            None => panic!(
                "current_thread_queue_len() must be called from within a Tokio runtime context"
            ),
        };

        let core = context.core.borrow();
        match core.as_ref() {
            Some(core) => core.tasks.len(),
            None => {
                panic!("current_thread_queue_len() called when scheduler core is not available")
            }
        }
    })
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

    fn new(ty: Option<masa::QueueType>, cap: usize) -> Self;

    fn sched_flavor(&self) -> SchedFlavor;
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