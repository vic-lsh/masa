use super::queue::{Queue, LocalRunQueue, PushError, PopError};
use super::Handle;
use crate::runtime::task;
use std::sync::Arc;

type Notified = task::Notified<Arc<Handle>>;

/// Timed Queue
pub(crate) struct TimedQueue{
    inner: LocalRunQueue<Notified>,
}

impl TimedQueue {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            inner: LocalRunQueue::with_capacity(cap),
        }
    }
}

impl Queue for TimedQueue {
    type Item = Notified;

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        let timer = item.timer();
        timer.set_enqueue_time();
        self.inner.push(item)
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        let item = self.inner.pop()?;
        item.timer().record_queue_lat();
        Ok(item)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }
}