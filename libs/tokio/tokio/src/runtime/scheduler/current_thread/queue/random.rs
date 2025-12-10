use std::collections::VecDeque;

use super::{IntoSchedFlavor, PopError, PushError, Queue, SchedFlavor};

struct XorRng {
    state: u64,
}

impl XorRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

pub(crate) struct RandomQueue<T> {
    inner: VecDeque<T>,
    push_count: u64,
    rng: XorRng,
}

impl<T> Queue for RandomQueue<T> {
    type Item = T;

    fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
            push_count: 0,
            rng: XorRng::new(0xDEADBEEF), // fixed seed for reproducibility
        }
    }

    fn push(&mut self, item: Self::Item) -> Result<(), PushError<Self::Item>> {
        self.inner.push_back(item);
        self.push_count += 1;

        Ok(())
    }

    fn pop(&mut self) -> Result<Self::Item, PopError> {
        if self.inner.is_empty() {
            return Err(PopError::Empty);
        }
        let shift = self.rng.next_u64() as usize % self.inner.len();
        self.inner.rotate_left(shift);
        self.inner.pop_front().ok_or(PopError::Empty)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_full(&self) -> bool {
        false
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.inner.capacity())
    }
}

impl<T> IntoSchedFlavor for RandomQueue<T> {
    fn into_sched_flavor() -> SchedFlavor {
        SchedFlavor::Random
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_queue_sched_flavor() {
        assert_eq!(FifoQueue::<u64>::into_sched_flavor(), SchedFlavor::Fifo);
    }
}
