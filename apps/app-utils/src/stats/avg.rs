use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub struct AvgTracker {
    sum: AtomicUsize,
    count: AtomicUsize,
}

impl AvgTracker {
    pub fn track(&self, fanout: usize) {
        self.sum.fetch_add(fanout, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self) -> usize {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            0
        } else {
            let sum = self.sum.load(Ordering::Relaxed);
            sum / count
        }
    }
}
