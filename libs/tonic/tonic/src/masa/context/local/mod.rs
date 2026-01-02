pub(crate) mod local;

use std::{collections::HashMap, sync::RwLock};

use masa::LatencyEstimator;

// TODO: tweak these values. should they be specific to each local priority selector?
pub(crate) const PERCENTILE: usize = 50;

fn estimate_method_latency<E: LatencyEstimator + Default + 'static>(
    map: &RwLock<HashMap<String, E>>,
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
        map.write().unwrap().insert(key.clone(), E::default());
    }
    None
}

// TODO: could reduce lock contention by giving each key it's own lock
fn track_method_latency<E: LatencyEstimator>(
    map: &RwLock<HashMap<String, E>>,
    key: String,
    duration: u64,
) {
    map.write().unwrap().get_mut(&key).unwrap().track(duration);
}
