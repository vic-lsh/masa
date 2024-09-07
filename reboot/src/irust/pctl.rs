use std::collections::VecDeque;

use env_logger::{Builder, Env};

struct PercentileTracker {
    capacity: usize,
    cur_queue: VecDeque<u64>,
    prev_queue: VecDeque<u64>,
    percentiles: Vec<u64>,
}

impl PercentileTracker {
    fn new(capacity: usize) -> Self {
        assert!(capacity >= 100, "Capacity should be no less than 100");
        PercentileTracker {
            capacity,
            cur_queue: VecDeque::with_capacity(capacity),
            prev_queue: VecDeque::with_capacity(capacity),
            percentiles: Vec::new(),
        }
    }

    fn add(&mut self, value: u64) {
        self.cur_queue.push_back(value);
        if self.cur_queue.len() >= self.capacity {
            self.calculate_percentiles();
            std::mem::swap(&mut self.cur_queue, &mut self.prev_queue);
            self.cur_queue.clear();
        }
    }

    fn calculate_percentiles(&mut self) {
        let mut all = Vec::new();
        all.extend(self.prev_queue.iter());
        all.extend(self.cur_queue.iter());
        all.sort();

        self.percentiles.clear();
        for i in 0..100 {
            let idx = (all.len() * i) / 100;
            self.percentiles.push(all[idx]);
        }
    }

    fn percentiles(&self) -> &Vec<u64> {
        &self.percentiles
    }

    fn percentile(&self, p: usize) -> u64 {
        assert!(p < 100, "Percentile should be less than 100");
        self.percentiles[p]
    }
}

fn main() {
    init_logging();

    let mut pctl = PercentileTracker::new(100);
    for i in 0..1000 {
        pctl.add(i);
    }
    log::info!("{:?}", pctl.percentiles());
    log::info!("{:?}", pctl.percentile(50));
    log::info!("{:?}", pctl.percentile(90));
    log::info!("{:?}", pctl.percentile(99));
}

fn init_logging() {
    Builder::from_env(Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} [{}:{}] {}",
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                // record.target(),
                record.args()
            )
        })
        .init();
    log::info!("Logging initialized");
}
