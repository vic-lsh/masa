pub(crate) mod local_direct;
pub(crate) mod local_indirect;

use std::{collections::HashMap, sync::RwLock};

pub(crate) use local_direct::LocalDeadlineDirect;
pub(crate) use local_indirect::LocalDeadlineIndirect;
use masa::LatencyDistribution;

// TODO: tweak these values. should they be specific to each local priority selector?
const DISTRIBUTION_CAPACITY: usize = 512;
const PERCENTILE: usize = 50;

fn estimate_method_latency(
    map: &RwLock<HashMap<String, LatencyDistribution>>,
    key: String,
) -> Option<u64> {
    let has_method = {
        let m = map.read().unwrap();
        let found = m.contains_key(&key);
        if found {
            let distribution = m.get(&key).unwrap();

            if distribution.can_estimate() {
                return Some(distribution.estimate(PERCENTILE));
            }
        }
        found
    };
    if !has_method {
        map.write().unwrap().insert(
            key.clone(),
            LatencyDistribution::new(key, DISTRIBUTION_CAPACITY),
        );
    }
    None
}

// TODO: could reduce lock contention by giving each key it's own lock
fn track_method_latency(
    map: &RwLock<HashMap<String, LatencyDistribution>>,
    key: String,
    duration: u64,
) {
    map.write().unwrap().get_mut(&key).unwrap().track(duration);
}
