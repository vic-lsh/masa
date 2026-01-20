use crate::tonic::child::child_client::ChildClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::masa_channel::LoadBalancedChannel;

/// Service registry that maps service IDs to client connections
pub struct ServiceRegistry {
    clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl ServiceRegistry {
    /// Create a new service registry
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a reference to the clients map (for bootstrap to update)
    pub fn clients(&self) -> Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>> {
        Arc::clone(&self.clients)
    }

    /// Get a cloned client for a service ID (for making calls)
    pub async fn get_client_clone(
        &self,
        service_id: &str,
    ) -> Option<ChildClient<LoadBalancedChannel>> {
        let guard = self.clients.read().await;
        guard.get(service_id).cloned()
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}
