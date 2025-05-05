mod local_direct;
mod local_indirect;

use std::{collections::HashMap, hash::Hash, sync::RwLock};

pub use local_direct::LocalDeadlineDirect;
pub use local_indirect::LocalDeadlineIndirect;
use masa::LatencyDistribution;

// TODO: tweak these values. should they be specific to each local priority selector?
const DISTRIBUTION_CAPACITY: usize = 1024;
const MIN_DISTRIBUTION_SIZE: usize = 500;
const PERCENTILE: usize = 50;

fn estimate_method_latency<K>(map: &RwLock<HashMap<K, LatencyDistribution>>, key: K) -> Option<u64>
where
    K: Hash + Eq + Copy,
{
    let has_method = map.read().unwrap().contains_key(&key);
    if !has_method {
        map.write()
            .unwrap()
            .insert(key, LatencyDistribution::new(DISTRIBUTION_CAPACITY));
    } else {
        let lock = map.read().unwrap();
        let distribution = lock.get(&key).unwrap();

        if distribution.len() >= MIN_DISTRIBUTION_SIZE {
            return Some(distribution.estimate(PERCENTILE));
        }
    }

    None
}

fn track_method_latency<K>(map: &RwLock<HashMap<K, LatencyDistribution>>, key: K, duration: u64)
where
    K: Hash + Eq,
{
    map.write().unwrap().get_mut(&key).unwrap().track(duration);
}
