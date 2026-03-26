use std::{collections::HashMap, sync::Arc, sync::Mutex, time::Duration};

use masa_core::LatencyEstimator;

use crate::masa::MethodRegistry;

#[derive(Debug)]
pub(crate) struct LatencyMap<E> {
    inner: Mutex<HashMap<u64, E>>,
}

impl<E> Default for LatencyMap<E>
where
    E: LatencyEstimator + Default + 'static,
{
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl<E> LatencyMap<E>
where
    E: LatencyEstimator + Default + 'static,
{
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get_estimate(&self, key: u64) -> Option<u64> {
        let mut m = self.inner.lock().unwrap();
        if let Some(estimator) = m.get(&key) {
            if estimator.can_estimate() {
                return Some(estimator.estimate());
            }
            // Found but cannot estimate yet
        } else {
            // Not found, insert default
            m.insert(key, E::default());
        }
        None
    }

    /// Returns the mean-only estimate (k=0), used for conservative early-return thresholds.
    pub(crate) fn get_mean_estimate(&self, key: u64) -> Option<u64> {
        let mut m = self.inner.lock().unwrap();
        if let Some(estimator) = m.get(&key) {
            if estimator.can_estimate() {
                return Some(estimator.mean_estimate());
            }
        } else {
            m.insert(key, E::default());
        }
        None
    }

    /// Returns the floor estimate, used for ER thresholds that are robust to mean inflation.
    pub(crate) fn get_mean_floor_estimate(&self, key: u64) -> Option<u64> {
        let mut m = self.inner.lock().unwrap();
        if let Some(estimator) = m.get(&key) {
            if estimator.can_estimate() {
                return Some(estimator.mean_floor_estimate());
            }
        } else {
            m.insert(key, E::default());
        }
        None
    }

    pub(crate) fn track(&self, key: u64, duration: u64) {
        let mut m = self.inner.lock().unwrap();
        let estimator = m.entry(key).or_insert_with(E::default);
        estimator.track(duration);
    }

    /// Iterates over the map and applies the given function to each entry.
    pub(crate) fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(u64, &E),
    {
        let guard = self.inner.lock().unwrap();
        for (&k, v) in guard.iter() {
            f(k, v);
        }
    }

    /// Checks if the map is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    #[cfg(test)]
    pub(crate) fn insert(&self, key: u64, value: E) {
        let mut m = self.inner.lock().unwrap();
        m.insert(key, value);
    }
}

/// Spawns a background task to periodically print latency estimates keyed by parent→child pair.
pub(crate) fn spawn_stats_printer<E: LatencyEstimator + Default + 'static>(
    distributions: Arc<LatencyMap<E>>,
    label: &'static str,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                if distributions.is_empty() {
                    continue;
                }

                let mut parts = Vec::new();
                distributions.for_each(|key, distribution| {
                    if distribution.can_estimate() {
                        let estimate = distribution.estimate();

                        // Decode key
                        let parent_id = key >> 32;
                        let child_id = key & 0xFFFFFFFF;

                        let registry = MethodRegistry::global();
                        let p_name = registry
                            .get_method_name(parent_id)
                            .map(|(s, m)| format!("{}::{}", s, m))
                            .unwrap_or_else(|| format!("{}", parent_id));
                        let c_name = registry
                            .get_method_name(child_id)
                            .map(|(s, m)| format!("{}::{}", s, m))
                            .unwrap_or_else(|| format!("{}", child_id));

                        parts.push(format!("{}=>{}: {} us", p_name, c_name, estimate));
                    } else {
                        parts.push(format!("{}: (no estimate)", key));
                    }
                });
                log::info!("{}: {}", label, parts.join(", "));
            }
        });
    }
}

/// Spawns a background task to periodically print latency estimates keyed by method ID only.
pub(crate) fn spawn_method_stats_printer<E: LatencyEstimator + Default + 'static>(
    distributions: Arc<LatencyMap<E>>,
    label: &'static str,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                if distributions.is_empty() {
                    continue;
                }

                let mut parts = Vec::new();
                distributions.for_each(|key, distribution| {
                    if distribution.can_estimate() {
                        let estimate = distribution.estimate();
                        let registry = MethodRegistry::global();
                        let name = registry
                            .get_method_name(key)
                            .map(|(s, m)| format!("{}::{}", s, m))
                            .unwrap_or_else(|| format!("{}", key));
                        parts.push(format!("{}: {} us", name, estimate));
                    } else {
                        parts.push(format!("{}: (no estimate)", key));
                    }
                });
                log::info!("{}: {}", label, parts.join(", "));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyRms;

    #[test]
    fn test_latency_map_basic_operations() {
        let map = LatencyMap::<LatencyRms>::new();
        let key = 12345u64;

        // Initially empty
        assert!(map.is_empty());
        // get_estimate creates the entry if not found, but returns None
        assert_eq!(map.get_estimate(key), None);
        // Now it is not empty because get_estimate created the entry
        assert!(!map.is_empty());

        // Inject an estimator with a short update interval (2) for testing.
        {
            map.insert(key, LatencyRms::new(2));
        }

        // 1st track: sum_sq=10000, count=1. No update.
        map.track(key, 100);
        // LatencyRms default can_estimate might be true returning 0 (cached)
        assert_eq!(map.get_estimate(key), Some(0));

        // 2nd track: sum_sq=20000, count=2. Update triggers.
        // RMS = 100.
        map.track(key, 100);
        assert_eq!(map.get_estimate(key), Some(100));

        // Test for_each
        let mut count = 0;
        map.for_each(|k, _v| {
            if k == key {
                count += 1;
            }
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn test_latency_map_concurrency() {
        use std::thread;

        let map = Arc::new(LatencyMap::<LatencyRms>::new());
        let key = 99999u64;

        // Insert with small window to ensure updates happen
        map.insert(key, LatencyRms::new(10));

        let mut handles = vec![];
        for _ in 0..10 {
            let map_clone = map.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    map_clone.track(key, 10);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Total 1000 tracks of value 10.
        // RMS should be 10.
        assert_eq!(map.get_estimate(key), Some(10));
    }
}
