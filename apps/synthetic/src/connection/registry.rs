use crate::error::{ConnectionError, Result};
use crate::tonic::child::child_client::ChildClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::masa_channel::LoadBalancedChannel;

/// Enhanced service registry that maps service IDs to client connections
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

    /// Get a cloned client with error handling
    pub async fn get_client(&self, service_id: &str) -> Result<ChildClient<LoadBalancedChannel>> {
        self.get_client_clone(service_id)
            .await
            .ok_or_else(|| ConnectionError::ServiceNotAvailable(service_id.to_string()).into())
    }

    /// Update the clients map (used by bootstrap)
    pub async fn update_clients(
        &self,
        new_clients: HashMap<String, ChildClient<LoadBalancedChannel>>,
    ) -> Result<()> {
        let mut guard = self.clients.write().await;
        let _ = std::mem::replace(&mut *guard, new_clients);
        Ok(())
    }

    /// Check if a service is connected
    pub async fn is_connected(&self, service_id: &str) -> bool {
        let guard = self.clients.read().await;
        guard.contains_key(service_id)
    }

    /// Get all connected service IDs
    pub async fn list_services(&self) -> Vec<String> {
        let guard = self.clients.read().await;
        guard.keys().cloned().collect()
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}
