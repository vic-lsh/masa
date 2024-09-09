use env_logger::{Builder, Env};

struct LatencyTracker {
    capacity: usize,
    cur_queue: Vec<u64>,
    prev_queue: Vec<u64>,
    mean: u64,
    percentiles: Vec<u64>,
}

impl LatencyTracker {
    fn new(capacity: usize) -> Self {
        assert!(capacity >= 100, "Capacity should be no less than 100");
        LatencyTracker {
            capacity,
            cur_queue: Vec::with_capacity(capacity),
            prev_queue: Vec::with_capacity(capacity),
            mean: 0,
            percentiles: Vec::new(),
        }
    }

    fn add(&mut self, value: u64) {
        self.cur_queue.push(value);
        if self.cur_queue.len() >= self.capacity {
            self.update();
            std::mem::swap(&mut self.cur_queue, &mut self.prev_queue);
            self.cur_queue.clear();
        }
    }

    fn update(&mut self) {
        let mut values = Vec::new();
        values.extend(self.prev_queue.iter());
        values.extend(self.cur_queue.iter());
        values.sort();

        let sum: u64 = values.iter().sum();
        self.mean = sum / values.len() as u64;

        self.percentiles.clear();
        for i in 0..100 {
            let idx = (values.len() * i) / 100;
            self.percentiles.push(values[idx]);
        }
    }

    fn percentiles(&self) -> &Vec<u64> {
        &self.percentiles
    }

    fn percentile(&self, p: usize) -> u64 {
        assert!(p < 100, "Percentile should be less than 100");
        self.percentiles[p]
    }

    fn estimate(&self) -> u64 {
        self.mean
    }
}

fn main() {
    init_logging();

    let mut pctl = LatencyTracker::new(100);
    for i in 0..1000 {
        pctl.add(i);
    }
    log::info!("{:?}", pctl.percentiles());
    log::info!("{:?}", pctl.percentile(50));
    log::info!("{:?}", pctl.percentile(90));
    log::info!("{:?}", pctl.percentile(99));
    log::info!("{:?}", pctl.estimate());
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
