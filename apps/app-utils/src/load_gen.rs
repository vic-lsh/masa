use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use structopt::StructOpt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Apis")]
    pub apis: Vec<String>,
    #[serde(rename = "Slos")]
    pub slos: Vec<u64>,
    #[serde(rename = "Rps")]
    pub rps_values: Vec<u64>,
    #[serde(rename = "Gap")]
    pub gap: String,
    #[serde(rename = "WarmupSecs")]
    pub warmup_secs: u64,
    #[serde(rename = "DurationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "Concurrency")]
    pub concurrency: usize,
    #[serde(rename = "Addr")]
    pub addr: String,
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct LoadGenArgs {
    #[structopt(long, required = true)]
    pub gen_config: PathBuf,
    #[structopt(long, required = true)]
    pub output_path: String,
    #[structopt(long)]
    pub save_logs: bool,
}

pub struct Counters {
    counters_map: HashMap<String, AtomicUsize>,
}

impl Clone for Counters {
    fn clone(&self) -> Self {
        let mut cloned = HashMap::new();
        for k in self.counters_map.keys() {
            cloned.insert(k.clone(), AtomicUsize::new(self.get(&k)));
        }
        Self {
            counters_map: cloned,
        }
    }
}

impl Counters {
    pub fn new(keys: &[&str]) -> Self {
        let mut map = HashMap::new();
        for k in keys {
            map.insert(k.to_string(), AtomicUsize::new(0));
        }
        Self { counters_map: map }
    }

    pub fn get(&self, k: &str) -> usize {
        self.counters_map.get(k).unwrap().load(Ordering::SeqCst)
    }

    pub fn increment(&self, k: &str) {
        self.counters_map
            .get(k)
            .unwrap()
            .fetch_add(1, Ordering::SeqCst);
    }
}
