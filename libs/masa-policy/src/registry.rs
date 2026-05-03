use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tonic_core::CowGrpcMethod;

/// Opaque method identifier. Only handed out by [`MethodRegistry`].
///
/// Process-local: two different processes may assign different `MethodId`s to
/// the same (service, method) pair. Use [`RootMethod`](masa_core::RootMethod)
/// for cross-process identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(u64);

/// Global registry for mapping (Service, Method) pairs to unique IDs.
/// This allows us to use u64 IDs in the hot path instead of hashing strings.
pub struct MethodRegistry {
    map: Mutex<HashMap<CowGrpcMethod, MethodId>>,
    service_map: Mutex<HashMap<Cow<'static, str>, MethodId>>,
    id_map: Mutex<HashMap<MethodId, CowGrpcMethod>>,
    next_id: AtomicU64,
}

impl fmt::Debug for MethodRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodRegistry")
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl MethodRegistry {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            service_map: Mutex::new(HashMap::new()),
            id_map: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Get the global instance of the method registry.
    pub fn global() -> &'static Self {
        static REGISTRY: OnceLock<MethodRegistry> = OnceLock::new();
        REGISTRY.get_or_init(MethodRegistry::new)
    }

    /// Get the ID for a (Service, Method) pair, registering it if it doesn't exist.
    pub fn get_or_register(&self, key: CowGrpcMethod) -> MethodId {
        // Fast path: check if already registered
        {
            let map = self.map.lock().unwrap();
            if let Some(&id) = map.get(&key) {
                return id;
            }
        }

        // Slow path: register
        let mut map = self.map.lock().unwrap();
        // Double check after re-acquiring lock
        if let Some(&id) = map.get(&key) {
            return id;
        }

        let id = MethodId(self.next_id.fetch_add(1, Ordering::Relaxed));
        map.insert(key.clone(), id);

        let mut id_map = self.id_map.lock().unwrap();
        id_map.insert(id, key);

        id
    }

    /// Get the ID for a service-only key.
    ///
    /// Fanout fallback estimates use service-shape signatures to avoid
    /// fragmenting on high-cardinality method/interface names. Reuse the
    /// method registry so the data path still deals in compact numeric IDs.
    pub fn get_or_register_service(&self, service: impl Into<Cow<'static, str>>) -> MethodId {
        self.get_or_register_service_cow(service.into())
    }

    /// Get the IDs for both exact method and service-only identity.
    ///
    /// Used by fanout estimation on the child-RPC data path. This avoids
    /// asking callers to perform a second method-registry lookup just to build
    /// the coarser service-shape signature.
    pub fn get_or_register_with_service(&self, key: CowGrpcMethod) -> (MethodId, MethodId) {
        let service_id = self.get_or_register_service_cow(Cow::Owned(key.service().to_string()));
        let method_id = self.get_or_register(key);
        (method_id, service_id)
    }

    fn get_or_register_service_cow(&self, service: Cow<'static, str>) -> MethodId {
        {
            let map = self.service_map.lock().unwrap();
            if let Some(&id) = map.get(service.as_ref()) {
                return id;
            }
        }

        let mut map = self.service_map.lock().unwrap();
        if let Some(&id) = map.get(service.as_ref()) {
            return id;
        }

        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = MethodId(id);
        map.insert(service.clone(), id);

        let mut id_map = self.id_map.lock().unwrap();
        id_map.insert(id, CowGrpcMethod::new(service, "*"));
        id
    }

    /// Reverse lookup: Get (Service, Method) from ID.
    pub fn get_method_name(&self, id: MethodId) -> Option<CowGrpcMethod> {
        let map = self.id_map.lock().unwrap();
        map.get(&id).cloned()
    }
}
