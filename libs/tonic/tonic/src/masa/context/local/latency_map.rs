use std::{collections::HashMap, sync::Mutex};

use masa_core::LatencyEstimator;

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
        use std::sync::Arc;
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
