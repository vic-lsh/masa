use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::fmt;

/// Global registry for mapping (Service, Method) pairs to unique IDs.
/// This allows us to use u64 IDs in the hot path instead of hashing strings.
pub struct MethodRegistry {
    map: Mutex<HashMap<(Cow<'static, str>, Cow<'static, str>), u64>>,
    id_map: Mutex<HashMap<u64, (Cow<'static, str>, Cow<'static, str>)>>,
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
    pub fn get_or_register_method(&self, service: &str, method: &str) -> u64 {
        // Optimized path: check if exists
        {
            let map = self.map.lock().unwrap();
            let key_ref = (Cow::Borrowed(service), Cow::Borrowed(method));
            if let Some(&id) = map.get(&key_ref) {
                return id;
            }
        }

        // Slow path: register (need to acquire both locks, or just sequentialize)
        // We use a separate lock scope or just lock one then the other.
        // Since we are single-threaded mostly, contention is low.
        // To be safe against deadlocks (though unlikely here with simple hierarchy),
        // we can just re-acquire map lock then id_map lock.
        
        let mut map = self.map.lock().unwrap();
        // Double check
        let key_ref = (Cow::Borrowed(service), Cow::Borrowed(method));
        if let Some(&id) = map.get(&key_ref) {
            return id;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Create owned copies for storage
        let s_owned: Cow<'static, str> = Cow::Owned(service.to_string());
        let m_owned: Cow<'static, str> = Cow::Owned(method.to_string());
        
        let key = (s_owned.clone(), m_owned.clone());
        map.insert(key, id);
        
        // Insert into reverse map
        let mut id_map = self.id_map.lock().unwrap();
        id_map.insert(id, (s_owned, m_owned));
        
        id
    }

    /// Reverse lookup: Get (Service, Method) from ID.
    /// Returns owned strings (clones) because we can't return references into the lock.
    pub fn get_method_name(&self, id: u64) -> Option<(String, String)> {
        let map = self.id_map.lock().unwrap();
        map.get(&id).map(|(s, m)| (s.to_string(), m.to_string()))
    }
}
