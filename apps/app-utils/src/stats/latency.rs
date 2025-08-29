use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

type LatencyUs = u64;

struct Record {
    id: usize,
    latency: LatencyUs,
}

pub struct SyncLatencyTracker {
    next_id: Arc<AtomicUsize>,
    records: UnboundedSender<Record>,
}

pub struct SyncLatencyConsumer {
    pub name: String,
    next_id: Arc<AtomicUsize>,
    records: UnboundedReceiver<Record>,
    tmp_consumed: Option<Record>,
}

pub struct LatencyDist {
    records: Vec<LatencyUs>,
    sorted: bool,
}

pub fn new_latency_tracker(name: impl Into<String>) -> (SyncLatencyTracker, SyncLatencyConsumer) {
    let next_id = Arc::new(AtomicUsize::new(0));
    let next_id_consumer = next_id.clone();
    let (tx, rx) = unbounded_channel();
    (
        SyncLatencyTracker {
            records: tx,
            next_id,
        },
        SyncLatencyConsumer {
            name: name.into(),
            records: rx,
            next_id: next_id_consumer,
            tmp_consumed: None,
        },
    )
}

impl SyncLatencyTracker {
    pub fn track(&self, latency: LatencyUs) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.records
            .send(Record { id, latency })
            .expect("channel closed unexpectedly");
    }
}

impl SyncLatencyConsumer {
    pub fn consume(&mut self) -> LatencyDist {
        // we only read records up to this ID (IDs are sequentially increasing).
        // Records are coming in as we consume, and we don't want to consume
        // records that arrive after we have started consuming.
        let max_id = self.next_id.load(Ordering::Relaxed);

        let mut records = Vec::new();
        if let Some(rec) = self.tmp_consumed.take() {
            records.push(rec.latency);
        }

        while let Ok(rec) = self.records.try_recv() {
            if rec.id >= max_id {
                self.tmp_consumed = Some(rec);
                break;
            }
            records.push(rec.latency);
        }

        LatencyDist::new(records)
    }
}

impl LatencyDist {
    pub fn percentile(&mut self, p: f64) -> LatencyUs {
        assert!(
            p >= 0.0 && p <= 100.0,
            "Percentile must be between 0 and 100"
        );

        let data = self.get_raw_dist();

        if data.is_empty() {
            return 0;
        }
        if data.len() == 1 {
            return data[0];
        }

        // Calculate the index
        // For percentile calculation, we use n = data.len() and k = p/100
        // Index = k * (n - 1)
        let k = p / 100.0;
        let n = data.len() as f64;
        let idx_f = k * (n - 1.0);

        // Get the integer and fractional parts of the index
        let idx_lower = idx_f.floor() as usize;
        let idx_upper = idx_f.ceil() as usize;
        let weight = idx_f - idx_f.floor();

        // Linear interpolation between the two nearest values
        if idx_lower == idx_upper {
            return data[idx_lower];
        } else {
            let lower_val = data[idx_lower] as f64;
            let upper_val = data[idx_upper] as f64;
            // TODO: revisit the rounding here.
            return (lower_val + weight * (upper_val - lower_val)) as LatencyUs;
        }
    }

    pub fn average(&self) -> f64 {
        self.records.iter().sum::<u64>() as f64 / self.records.len() as f64
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }
}

impl LatencyDist {
    fn new(records: impl Into<Vec<LatencyUs>>) -> Self {
        Self {
            records: records.into(),
            sorted: false,
        }
    }

    fn get_raw_dist(&mut self) -> &[LatencyUs] {
        if !self.sorted {
            self.records.sort();
            self.sorted = true;
        }
        &self.records
    }
}

pub struct StatsTracker {
    trackers: HashMap<String, SyncLatencyTracker>,
}

impl StatsTracker {
    pub fn new(keys: Vec<&str>, print: bool) -> Self {
        let mut trackers = HashMap::new();
        let mut consumers = HashMap::new();
        for k in &keys {
            let (tracker, consumer) = new_latency_tracker(k.to_string());
            trackers.insert(k.to_string(), tracker);
            consumers.insert(k.to_string(), consumer);
        }

        tokio::spawn(async move {
            const PERCENTILES: [f64; 4] = [50.0, 90.0, 99.0, 99.9];
            const DELTA_MS: u64 = 1000;

            let name_width = consumers
                .iter()
                .map(|(k, _)| k.len())
                .max()
                .expect("max must exist")
                + 4;

            let mut millis = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(DELTA_MS)).await;
                if !print {
                    for (_k, c) in consumers.iter_mut() {
                        let _ = c.consume();
                    }
                    continue;
                };
                millis += DELTA_MS;
                println!("#{}", millis);
                for (k, c) in consumers.iter_mut() {
                    let mut dist = c.consume();
                    print!("{: <width$}", k, width = name_width);
                    print!("# recs {: <width$}", dist.len(), width = 6);
                    print!("avg: {} ", dist.average());
                    for p in PERCENTILES {
                        print!("p{}: {} ", p, dist.percentile(p));
                    }
                    print!("\n");
                }
                print!("\n");
            }
        });

        Self { trackers }
    }

    pub fn get(&self, k: &str) -> &SyncLatencyTracker {
        self.trackers.get(k).unwrap()
    }
}

#[cfg(test)]
mod tests {}
