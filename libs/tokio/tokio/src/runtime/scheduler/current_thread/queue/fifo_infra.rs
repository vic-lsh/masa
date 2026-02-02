use std::collections::VecDeque;

use masa_core::{Prioritize, PriorityHint};

use super::{IntoSchedFlavor, PopError, PushError, Queue, SchedFlavor};

pub(crate) struct FifoInfraQueue<T> {
    infra_q: VecDeque<T>,
    other_q: VecDeque<T>,
    push_count: u64,
}

impl<T: Prioritize> Queue for FifoInfraQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            infra_q: VecDeque::with_capacity(cap),
            other_q: VecDeque::with_capacity(cap),
            push_count: 0,
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        if item.priority() == PriorityHint::infra() {
            self.infra_q.push_back(item);
        } else {
            self.other_q.push_back(item);
        }
        self.push_count += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        if let Some(item) = self.infra_q.pop_front() {
            return Ok(item);
        }
        if let Some(item) = self.other_q.pop_front() {
            return Ok(item);
        }
        Err(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.infra_q.len() + self.other_q.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.infra_q.capacity() + self.other_q.capacity())
    }
}

impl<T> IntoSchedFlavor for FifoInfraQueue<T> {
    fn into_sched_flavor() -> SchedFlavor {
        SchedFlavor::Fifo
    }
}
