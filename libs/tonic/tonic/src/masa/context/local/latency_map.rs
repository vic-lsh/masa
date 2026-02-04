use std::{
    collections::HashMap,
    sync::{Mutex, RwLock},
};

use masa_core::LatencyEstimator;

#[derive(Debug)]
pub(crate) struct LatencyMap<K, E> {
    inner: RwLock<HashMap<K, Mutex<E>>>,
}

impl<K, E> Default for LatencyMap<K, E>
where
    K: std::hash::Hash + Eq + Clone,
    E: LatencyEstimator + Default + 'static,
{
    fn default() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

impl<K, E> LatencyMap<K, E>
where
    K: std::hash::Hash + Eq + Clone,
    E: LatencyEstimator + Default + 'static,
{
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get_estimate(&self, key: &K) -> Option<u64> {
        // Optimistic read
        let has_method = {
            let m = self.inner.read().unwrap();
            if let Some(distribution_lock) = m.get(key) {
                let distribution = distribution_lock.lock().unwrap();
                if distribution.can_estimate() {
                    return Some(distribution.estimate());
                }
                true // Found but cannot estimate yet
            } else {
                false // Not found
            }
        };

        if !has_method {
            // Double-checked locking could be done here, but insert is idempotent-ish enough
            // (worst case we overwrite or just get the entry)
            let mut m = self.inner.write().unwrap();
            m.entry(key.clone())
                .or_insert_with(|| Mutex::new(E::default()));
        }
        None
    }

    pub(crate) fn track(&self, key: K, duration: u64) {
        // Optimistic read
        {
            let m = self.inner.read().unwrap();
            if let Some(estimator) = m.get(&key) {
                let mut est = estimator.lock().unwrap();
                est.track(duration);
                return;
            }
        } // drop read lock

        // Write path
        let mut m = self.inner.write().unwrap();
        let estimator = m.entry(key).or_insert_with(|| Mutex::new(E::default()));
        let mut est = estimator.lock().unwrap();
        est.track(duration);
    }

    /// Iterates over the map and applies the given function to each entry.
    /// This holds a read lock on the map for the duration of the iteration,
    /// and temporarily locks each entry's mutex.
    pub(crate) fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&K, &E),
    {
        let guard = self.inner.read().unwrap();
        for (k, v) in guard.iter() {
            let est = v.lock().unwrap();
            f(k, &*est);
        }
    }

    /// Checks if the map is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }

    #[cfg(test)]
    pub(crate) fn insert(&self, key: K, value: E) {
        let mut m = self.inner.write().unwrap();
        m.insert(key, Mutex::new(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyRms;

    #[test]
    fn test_latency_map_basic_operations() {
        let map = LatencyMap::<String, LatencyRms>::new();
        let key = "method1".to_string();

        // Initially empty
        assert!(map.is_empty());
        // get_estimate creates the entry if not found, but returns None
        assert_eq!(map.get_estimate(&key), None);
        // Now it is not empty because get_estimate created the entry
        assert!(!map.is_empty());

        // Inject an estimator with a short update interval (2) for testing.
        {
            map.insert(key.clone(), LatencyRms::new(2));
        }

        // 1st track: sum_sq=10000, count=1. No update.
        map.track(key.clone(), 100);
        // LatencyRms default can_estimate might be true returning 0 (cached)
        assert_eq!(map.get_estimate(&key), Some(0));

        // 2nd track: sum_sq=20000, count=2. Update triggers.
        // RMS = 100.
        map.track(key.clone(), 100);
        assert_eq!(map.get_estimate(&key), Some(100));

        // Test for_each
        let mut count = 0;
        map.for_each(|k, _v| {
            if k == &key {
                count += 1;
            }
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn test_latency_map_concurrency() {
        use std::sync::Arc;
        use std::thread;

        let map = Arc::new(LatencyMap::<String, LatencyRms>::new());
        let key = "concurrent_key".to_string();

        // Insert with small window to ensure updates happen
        map.insert(key.clone(), LatencyRms::new(10));

        let mut handles = vec![];
        for _ in 0..10 {
            let map_clone = map.clone();
            let key_clone = key.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    map_clone.track(key_clone.clone(), 10);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Total 1000 tracks of value 10.
        // RMS should be 10.
        assert_eq!(map.get_estimate(&key), Some(10));
    }
}

