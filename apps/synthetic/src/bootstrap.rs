use crate::tonic::child::child_client::ChildClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::{JoinHandle, JoinSet};
use tonic::transport::masa_channel::LoadBalancedChannel;
use tracing::info;

/// Bootstrap task that connects to children asynchronously
pub struct ConnectionBootstrap {
    services: Vec<(String, String, u8)>, // (service_id, hostname_base, replicas)
    clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

pub struct ConnectionBootstrapTask {
    handle: JoinHandle<()>,
}

impl Drop for ConnectionBootstrapTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl ConnectionBootstrap {
    pub fn new(
        services: Vec<(String, String, u8)>,
        clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
    ) -> Self {
        Self { services, clients }
    }

    pub fn spawn(self) -> ConnectionBootstrapTask {
        let handle = tokio::spawn(async move {
            let ConnectionBootstrap { services, clients } = self;

            info!(
                count = services.len(),
                "Connecting to children asynchronously"
            );
            let connected_clients = Self::connect_to_children(services).await;

            info!("Children connected");

            let mut guard = clients.write().await;
            let _ = std::mem::replace(&mut *guard, connected_clients);
        });

        ConnectionBootstrapTask { handle }
    }

    async fn connect_to_children(
        services: Vec<(String, String, u8)>,
    ) -> HashMap<String, ChildClient<LoadBalancedChannel>> {
        let mut tasks = JoinSet::new();
        for (service_id, hostname_base, replicas) in services {
            tasks.spawn(async move {
                let client = ChildClient::new(
                    LoadBalancedChannel::new_from(
                        hostname_base,
                        8000,
                        replicas,
                        1, // Docker Compose starts replica numbering from 1
                    )
                    .await,
                );
                (service_id, client)
            });
        }

        let mut clients = HashMap::with_capacity(tasks.len());
        while let Some(task_result) = tasks.join_next().await {
            let (service_id, client) = task_result.unwrap_or_else(|e| {
                panic!("Connection bootstrap task failed: {}", e);
            });
            info!(service_id = %service_id, "Connected to child service");
            clients.insert(service_id, client);
        }
        clients
    }
}
