mod fifo;
mod fifo_infra;

#[cfg(any(
    feature = "prio_global",
    feature = "prio_global_queue_tracing",
    feature = "prio_local",
    feature = "prio_oldest",
))]
mod prio_bh;

#[cfg(any(feature = "prio_oldest"))]
mod prio_bh_rr;

#[cfg(any(
    feature = "fifo_span_tracing",
    feature = "fifo_queue_tracing",
    feature = "prio_global_queue_tracing",
))]
mod timed;

#[cfg(any(
    feature = "fifo_span_tracing",
    feature = "fifo_queue_tracing",
    feature = "prio_global_queue_tracing",
))]
pub(crate) type LocalRunQueue<T> = timed::TimedQueue<LocalRunQueueInner<T>>;

#[cfg(not(any(
    feature = "fifo_span_tracing",
    feature = "fifo_queue_tracing",
    feature = "prio_global_queue_tracing",
)))]
pub(crate) type LocalRunQueue<T> = LocalRunQueueInner<T>;

#[cfg(not(any(
    feature = "prio_global",
    feature = "prio_global_queue_tracing",
    feature = "prio_local",
    feature = "prio_oldest",
)))]
pub(crate) type LocalRunQueueInner<T> = fifo::FifoQueue<T>;

#[cfg(any(
    feature = "prio_global",
    feature = "prio_global_queue_tracing",
    feature = "prio_local",
))]
pub(crate) type LocalRunQueueInner<T> = prio_bh::BinaryHeapQueue<T>;

#[cfg(any(feature = "prio_oldest"))]
pub(crate) type LocalRunQueueInner<T> = prio_bh_rr::BinaryHeapRoundRobinQueue<T>;

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

/// Get the current queue length for the current_thread runtime.
///
/// # Panics
///
/// This function will panic if:
/// - Called from outside a Tokio runtime context
/// - Called from a multi-threaded runtime (only works with current_thread runtime)
/// - Called when the scheduler core is not available (e.g., outside of `block_on`)
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
