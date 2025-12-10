pub(crate) mod local_direct;
pub(crate) mod local_indirect;
pub(crate) mod local_learned;

use std::{collections::HashMap, sync::RwLock};

#[allow(unused_imports)]
pub(crate) use local_direct::LocalDeadlineDirect;
#[allow(unused_imports)]
pub(crate) use local_indirect::LocalDeadlineIndirect;
#[allow(unused_imports)]
pub(crate) use local_learned::LocalDeadlineLearned;

use masa::LatencyDistribution;

// TODO: tweak these values. should they be specific to each local priority selector?
const DISTRIBUTION_CAPACITY: usize = 512;
const PERCENTILE: usize = 50;

#[allow(dead_code)]
fn estimate_method_latency(
    map: &RwLock<HashMap<String, LatencyDistribution>>,
    key: String,
) -> Option<u64> {
    if let Some(distribution) = map.read().unwrap().get(&key) {
        if distribution.can_estimate() {
            return Some(distribution.percentile(PERCENTILE));
        }
    } else {
        map.write().unwrap().insert(
            key.clone(),
            LatencyDistribution::new(key, DISTRIBUTION_CAPACITY),
        );
    }
    None
}

// TODO: could reduce lock contention by giving each key it's own lock
// TODO: Cleanup this code
#[allow(dead_code)]
fn track_method_latency(
    map: &RwLock<HashMap<String, LatencyDistribution>>,
    key: String,
    duration: u64,
) {
    map.write().unwrap().get_mut(&key).unwrap().track(duration);
}
