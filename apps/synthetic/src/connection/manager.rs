// Unified connection manager for synthetic application

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tracing::info;

use super::{bootstrap::ConnectionBootstrap, docker::resolve_hostnames};
use crate::constants::*;
use crate::error::{ConnectionError, Result};
use crate::tonic::child::child_client::ChildClient;

/// Connection mode type
#[derive(Debug, Clone)]
pub enum ConnectionMode {
    /// Hard-coded mode: Traditional hop-based service communication
    HardCode {
        child_services: Vec<crate::config::ChildService>,
    },
    /// Call graph mode: Dynamic service discovery and method calls
    CallGraph {
        services: Vec<crate::config::ServiceDefinition>,
    },
}

/// Unified connection manager
pub struct ConnectionManager {
    mode: ConnectionMode,
    clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
    port: u16,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new(mode: ConnectionMode, port: u16) -> Self {
        Self {
            mode,
            clients: Arc::new(RwLock::new(HashMap::new())),
            port,
        }
    }

    /// Initialize all connections based on mode
    pub async fn initialize_connections(&self) -> Result<()> {
        match &self.mode {
            ConnectionMode::HardCode { child_services } => {
                self.initialize_hard_code_connections(child_services)
                    .await?
            }
            ConnectionMode::CallGraph { services } => {
                self.initialize_call_graph_connections(services).await?
            }
        }
        Ok(())
    }

    /// Initialize connections for hard_code mode
    async fn initialize_hard_code_connections(
        &self,
        child_services: &[crate::config::ChildService],
    ) -> Result<()> {
        let mut services = Vec::new();
        let mut service_map = HashMap::new();

        for (index, service) in child_services.iter().enumerate() {
            let hostname = crate::connection::docker::resolve_child_hostname(
                index as u32,
                DOCKER_REPLICA_START,
            );

            let client = ChildClient::new(
                LoadBalancedChannel::new_from(
                    hostname,
                    self.port,
                    service.replicas,
                    DOCKER_REPLICA_START as u8,
                )
                .await,
            );

            service_map.insert(service.id.clone(), index);
            services.push(client);
        }

        // Update clients with hard_code connections
        let mut guard = self.clients.write().await;
        for (index, client) in services.into_iter().enumerate() {
            guard.insert(format!("service_{}", index), client);
        }

        info!(
            "Initialized {} hard_code child services on port {}",
            child_services.len(),
            self.port
        );
        Ok(())
    }

    /// Initialize connections for call graph mode
    async fn initialize_call_graph_connections(
        &self,
        services: &[crate::config::ServiceDefinition],
    ) -> Result<()> {
        let mut services_to_connect = Vec::new();

        for service in services {
            let hostname_base = resolve_hostnames(&service.id, service.replicas)?;
            services_to_connect.push((service.id.clone(), hostname_base, service.replicas));
        }

        // Spawn bootstrap task to connect asynchronously
        if !services_to_connect.is_empty() {
            let bootstrap =
                ConnectionBootstrap::new(services_to_connect, Arc::clone(&self.clients));
            bootstrap.spawn().await?;
        }

        info!(
            "Started bootstrap for {} call graph services on port {}",
            services.len(),
            self.port
        );
        Ok(())
    }

    /// Get a client by service ID (for call graph mode)
    pub async fn get_client(&self, service_id: &str) -> Result<ChildClient<LoadBalancedChannel>> {
        let guard = self.clients.read().await;
        guard
            .get(service_id)
            .cloned()
            .ok_or_else(|| ConnectionError::ServiceNotAvailable(service_id.to_string()).into())
    }

    /// Get a client by index (for hard_code mode)
    pub async fn get_client_by_index(
        &self,
        index: usize,
    ) -> Result<ChildClient<LoadBalancedChannel>> {
        let key = format!("service_{}", index);
        let guard = self.clients.read().await;
        guard
            .get(&key)
            .cloned()
            .ok_or_else(|| ConnectionError::ServiceNotAvailable(key).into())
    }

    /// Get current connection mode
    pub fn mode(&self) -> &ConnectionMode {
        &self.mode
    }
}
