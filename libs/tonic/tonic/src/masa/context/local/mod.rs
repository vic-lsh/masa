pub(crate) mod local;

use std::{
    collections::HashMap,
    sync::{Mutex, RwLock},
};

use masa_core::LatencyEstimator;

// TODO: tweak these values. should they be specific to each local priority selector?
pub(crate) const PERCENTILE: usize = 50;

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
                    return Some(distribution.estimate(PERCENTILE));
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
