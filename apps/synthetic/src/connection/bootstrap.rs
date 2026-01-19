// Asynchronous connection bootstrap for synthetic application

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tracing::info;

use crate::constants::DOCKER_REPLICA_START;
use crate::error::Result;
use crate::tonic::child::child_client::ChildClient;

/// Bootstrap task that connects to children asynchronously
pub struct ConnectionBootstrap {
    services: Vec<(String, String, u8)>, // (service_id, hostname_base, replicas)
    clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl ConnectionBootstrap {
    /// Create a new bootstrap instance
    pub fn new(
        services: Vec<(String, String, u8)>,
        clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
    ) -> Self {
        Self { services, clients }
    }

    /// Spawn bootstrap task
    pub async fn spawn(self) -> Result<()> {
        tokio::spawn(async move {
            if let Err(e) = self.connect_to_children().await {
                tracing::error!("Bootstrap failed: {}", e);
            }
        });
        Ok(())
    }

    /// Connect to all child services
    async fn connect_to_children(self) -> Result<()> {
        info!(
            count = self.services.len(),
            "Connecting to children asynchronously"
        );

        let connected_clients = Self::establish_connections(self.services).await?;

        info!("All children connected successfully");

        // Update clients map
        let mut guard = self.clients.write().await;
        let _ = std::mem::replace(&mut *guard, connected_clients);

        Ok(())
    }

    /// Establish connections to all specified services
    async fn establish_connections(
        services: Vec<(String, String, u8)>,
    ) -> Result<HashMap<String, ChildClient<LoadBalancedChannel>>> {
        let mut clients = HashMap::new();

        for (service_id, hostname_base, replicas) in services {
            let client = ChildClient::new(
                LoadBalancedChannel::new_from(
                    hostname_base,
                    8000, // Default port
                    replicas,
                    DOCKER_REPLICA_START as u8,
                )
                .await,
            );

            info!(service_id = %service_id, "Connected to child service");
            clients.insert(service_id, client);
        }

        Ok(clients)
    }
}
