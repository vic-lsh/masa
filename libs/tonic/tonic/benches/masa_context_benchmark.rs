use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

// ============================================================================
// OLD IMPLEMENTATION SIMULATION
// ============================================================================

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct OldCowGrpcMethod {
    service: Cow<'static, str>,
    method: Cow<'static, str>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct OldParentToChildId {
    parent: OldCowGrpcMethod,
    child: OldCowGrpcMethod,
}

struct OldLatencyMap<E> {
    inner: RwLock<HashMap<OldParentToChildId, Mutex<E>>>,
}

impl<E: Default + Clone> OldLatencyMap<E> {
    fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    fn track(&self, key: OldParentToChildId, duration: u64) {
        // Optimistic read
        {
            let m = self.inner.read().unwrap();
            if let Some(estimator) = m.get(&key) {
                let mut est = estimator.lock().unwrap();
                *est = E::default(); // Simulate tracking
                return;
            }
        } // drop read lock

        // Write path
        let mut m = self.inner.write().unwrap();
        let estimator = m.entry(key).or_insert_with(|| Mutex::new(E::default()));
        let mut est = estimator.lock().unwrap();
        *est = E::default();
    }
}

// ============================================================================
// NEW IMPLEMENTATION SIMULATION (Mechanisms only)
// ============================================================================

struct MethodRegistry {
    map: Mutex<HashMap<(Cow<'static, str>, Cow<'static, str>), u64>>,
    next_id: AtomicU64,
}

impl MethodRegistry {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn global() -> &'static Self {
        static REGISTRY: OnceLock<MethodRegistry> = OnceLock::new();
        REGISTRY.get_or_init(MethodRegistry::new)
    }

    fn get_or_register_method(&self, service: &str, method: &str) -> u64 {
        {
            let map = self.map.lock().unwrap();
            let key_ref = (Cow::Borrowed(service), Cow::Borrowed(method));
            if let Some(&id) = map.get(&key_ref) {
                return id;
            }
        }

        let mut map = self.map.lock().unwrap();
        let key_ref = (Cow::Borrowed(service), Cow::Borrowed(method));
        if let Some(&id) = map.get(&key_ref) {
            return id;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let key_owned = (
            Cow::Owned(service.to_string()),
            Cow::Owned(method.to_string()),
        );
        map.insert(key_owned, id);
        id
    }
}

struct NewLatencyMap<E> {
    inner: Mutex<HashMap<u64, E>>,
}

impl<E: Default + Clone> NewLatencyMap<E> {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn track(&self, key: u64, _duration: u64) {
        let mut m = self.inner.lock().unwrap();
        let estimator = m.entry(key).or_insert_with(E::default);
        *estimator = E::default(); // Simulate tracking
    }
}

// Dummy estimator
#[derive(Default, Clone)]
struct Estimator;

// ============================================================================
// BENCHMARKS
// ============================================================================

fn bench_hot_path_old(c: &mut Criterion) {
    let map = Arc::new(OldLatencyMap::<Estimator>::new());

    // Simulate resolving method names (which involves allocation if overrides exist)
    let s_p = "ServiceParent";
    let m_p = "MethodParent";
    let s_c = "ServiceChild";
    let m_c = "MethodChild";

    c.bench_function("old_hot_path_track", |b| {
        b.iter(|| {
            // In the old path, every child RPC setup involved creating ParentToChildId
            // If overrides are used, strings are allocated.
            // We simulate the "common case" where they might be Cows but cloned.
            // Even if borrowed, hashing is done every time.

            let parent = OldCowGrpcMethod {
                service: Cow::Borrowed(black_box(s_p)),
                method: Cow::Borrowed(black_box(m_p)),
            };
            let child = OldCowGrpcMethod {
                service: Cow::Borrowed(black_box(s_c)),
                method: Cow::Borrowed(black_box(m_c)),
            };
            let key = OldParentToChildId { parent, child };

            map.track(key, 100);
        })
    });
}

fn bench_hot_path_new(c: &mut Criterion) {
    let map = Arc::new(NewLatencyMap::<Estimator>::new());
    let registry = MethodRegistry::global();

    // Pre-register (startup time)
    let p_id = registry.get_or_register_method("ServiceParent", "MethodParent");
    let c_id = registry.get_or_register_method("ServiceChild", "MethodChild");

    // In the new path, ParentID is resolved once per parent request (cached in struct).
    // ChildID is resolved once per child RPC call (cached in registry lookup).
    // But Registry lookup is also optimized.
    // However, the *LatencyMap* access itself uses the u64 key.

    c.bench_function("new_hot_path_track", |b| {
        b.iter(|| {
            // 1. Resolve Child (simulating registry lookup if not cached locally,
            // but in reality we do lookup every time in before_child_rpc)
            // But wait, my implementation of `before_child_rpc` calls `MethodRegistry::get_or_register_method` every time!
            // So we MUST include that in the bench to be fair.

            let resolved_child_id = registry
                .get_or_register_method(black_box("ServiceChild"), black_box("MethodChild"));

            // 2. Combine keys
            let key = (p_id << 32) | resolved_child_id;

            // 3. Track
            map.track(key, 100);
        })
    });
}

fn bench_hot_path_new_optimized_cached(c: &mut Criterion) {
    // This benchmark simulates if we were to cache the child ID in the client stub (future optimization).
    // Currently purely testing the map + lock speed.
    let map = Arc::new(NewLatencyMap::<Estimator>::new());
    let registry = MethodRegistry::global();
    let p_id = registry.get_or_register_method("ServiceParent", "MethodParent");
    let c_id = registry.get_or_register_method("ServiceChild", "MethodChild");
    let key = (p_id << 32) | c_id;

    c.bench_function("new_hot_path_track_cached_id", |b| {
        b.iter(|| {
            map.track(black_box(key), 100);
        })
    });
}

criterion_group!(
    benches,
    bench_hot_path_old,
    bench_hot_path_new,
    bench_hot_path_new_optimized_cached
);
criterion_main!(benches);
