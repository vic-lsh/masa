use std::hash::Hash;
use std::{collections::HashMap, fmt, sync::Mutex};

use masa_core::LatencyEstimator;

use crate::registry::MethodId;
use crate::MethodRegistry;

// ---------------------------------------------------------------------------
// Key types
// ---------------------------------------------------------------------------

/// Single method ID key (for `est_accumulated_cost`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MethodKey(pub(crate) MethodId);

/// Builder intermediates for [`ParentToChildKey`].
pub(crate) struct ParentToChildKeyRootStage(MethodId);
pub(crate) struct ParentToChildKeyParentStage(MethodId, MethodId);

/// (root API, parent, child) RPC method triple key (for remaining-wallclock,
/// child-wallclock estimates). Root API is the primary index so observations
/// from different ingress API types are not pooled together on a shared
/// downstream edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ParentToChildKey(MethodId, MethodId, MethodId);

impl ParentToChildKey {
    pub(crate) fn root_rpc_method(id: MethodId) -> ParentToChildKeyRootStage {
        ParentToChildKeyRootStage(id)
    }

    pub(crate) fn root(&self) -> MethodId {
        self.0
    }

    pub(crate) fn parent(&self) -> MethodId {
        self.1
    }

    pub(crate) fn child(&self) -> MethodId {
        self.2
    }
}

impl ParentToChildKeyRootStage {
    pub(crate) fn parent_rpc_method(self, id: MethodId) -> ParentToChildKeyParentStage {
        ParentToChildKeyParentStage(self.0, id)
    }
}

impl ParentToChildKeyParentStage {
    pub(crate) fn child_rpc_method(self, id: MethodId) -> ParentToChildKey {
        ParentToChildKey(self.0, self.1, id)
    }
}

impl fmt::Display for ParentToChildKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}]{}=>{}",
            format_method_name(self.root()),
            format_method_name(self.parent()),
            format_method_name(self.child())
        )
    }
}

/// Builder intermediate for [`RootToLocalKey`].
pub(crate) struct RootToLocalKeyBuilder(MethodId);

/// Root→local RPC method pair key (for `est_method_latency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RootToLocalKey(MethodId, MethodId);

impl RootToLocalKey {
    pub(crate) fn root_rpc_method(id: MethodId) -> RootToLocalKeyBuilder {
        RootToLocalKeyBuilder(id)
    }

    pub(crate) fn root(&self) -> MethodId {
        self.0
    }

    pub(crate) fn local(&self) -> MethodId {
        self.1
    }
}

impl RootToLocalKeyBuilder {
    pub(crate) fn local_rpc_method(self, id: MethodId) -> RootToLocalKey {
        RootToLocalKey(self.0, id)
    }
}

impl fmt::Display for RootToLocalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}=>{}",
            format_method_name(self.root()),
            format_method_name(self.local())
        )
    }
}

// ---------------------------------------------------------------------------
// LatencyMap
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct LatencyMap<K, E> {
    inner: Mutex<HashMap<K, E>>,
}

impl<K, E> Default for LatencyMap<K, E>
where
    K: Copy + Eq + Hash + 'static,
    E: LatencyEstimator + Default + 'static,
{
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl<K, E> LatencyMap<K, E>
where
    K: Copy + Eq + Hash + 'static,
    E: LatencyEstimator + Default + 'static,
{
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Get an estimate using the given extraction function, inserting a default entry if missing.
    fn get_estimate_with(&self, key: K, extract: impl FnOnce(&E) -> u64) -> Option<u64> {
        let mut m = self.inner.lock().unwrap();
        if let Some(estimator) = m.get(&key) {
            if estimator.can_estimate() {
                return Some(extract(estimator));
            }
        } else {
            m.insert(key, E::default());
        }
        None
    }

    pub(crate) fn get_estimate(&self, key: K) -> Option<u64> {
        self.get_estimate_with(key, E::estimate)
    }

    /// Returns the mean-only estimate (k=0), used for conservative early-return thresholds.
    #[allow(dead_code)]
    pub(crate) fn get_mean_estimate(&self, key: K) -> Option<u64> {
        self.get_estimate_with(key, E::mean_estimate)
    }

    /// Returns the floor estimate, used for ER thresholds that are robust to mean inflation.
    #[allow(dead_code)]
    pub(crate) fn get_mean_floor_estimate(&self, key: K) -> Option<u64> {
        self.get_estimate_with(key, E::mean_floor_estimate)
    }

    pub(crate) fn track(&self, key: K, duration: u64) {
        let mut m = self.inner.lock().unwrap();
        let estimator = m.entry(key).or_insert_with(E::default);
        estimator.track(duration);
    }

    /// Iterates over the map and applies the given function to each entry.
    pub(crate) fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&K, &E),
    {
        let guard = self.inner.lock().unwrap();
        for (k, v) in guard.iter() {
            f(k, v);
        }
    }

    /// Checks if the map is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    #[cfg(test)]
    pub(crate) fn insert(&self, key: K, value: E) {
        let mut m = self.inner.lock().unwrap();
        m.insert(key, value);
    }
}

// ---------------------------------------------------------------------------
// Stats printers
// ---------------------------------------------------------------------------

pub(crate) fn format_method_name(id: MethodId) -> String {
    MethodRegistry::global()
        .get_method_name(id)
        .map(|m| format!("{}::{}", m.service(), m.method()))
        .unwrap_or_else(|| format!("{:?}", id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyRms;
    use std::sync::Arc;
    use tonic_core::CowGrpcMethod;

    fn test_method_id() -> MethodId {
        MethodRegistry::global().get_or_register(CowGrpcMethod::new("TestService", "TestMethod"))
    }

    #[test]
    fn test_latency_map_basic_operations() {
        let map = LatencyMap::<MethodKey, LatencyRms>::new();
        let key = MethodKey(test_method_id());

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
            if *k == key {
                count += 1;
            }
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn test_latency_map_concurrency() {
        use std::thread;

        let map = Arc::new(LatencyMap::<MethodKey, LatencyRms>::new());
        let key = MethodKey(test_method_id());

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
