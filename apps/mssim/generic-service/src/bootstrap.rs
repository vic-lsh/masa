use crate::client_registry::ClientRegistry;
use crate::service_stubs::service_client::ServiceClient;
use crate::RpcClient;
use anyhow::{Context, Result};
use masa::transport::LoadBalancedChannel;
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::ServiceName;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::info;

pub(crate) struct ConnectionBootstrap {
    children: Vec<ServiceName>,
    children_for_log: Vec<(ServiceName, u64)>,
    deployment: Deployment,
    registry: Arc<ClientRegistry>,
}

pub(crate) struct ConnectionBootstrapTask {
    handle: JoinHandle<()>,
}

impl ConnectionBootstrapTask {
    /// Consume the task, yielding its `JoinHandle` so the caller can `.await` it.
    pub(crate) fn into_handle(self) -> JoinHandle<()> {
        self.handle
    }
}

impl ConnectionBootstrap {
    pub(crate) fn new(
        children: Vec<ServiceName>,
        children_for_log: Vec<(ServiceName, u64)>,
        deployment: Deployment,
        registry: Arc<ClientRegistry>,
    ) -> Self {
        Self {
            children,
            children_for_log,
            deployment,
            registry,
        }
    }

    /// Spawn bootstrap as a background task. Must be spawned (not awaited
    /// inline) to avoid deadlocks when two services need to connect to each
    /// other. On success, `ClientRegistry::install` publishes the map *and*
    /// flips the ready flag atomically, so post-bootstrap fanout misses
    /// surface as panics instead of silently succeeding.
    pub(crate) fn spawn(self) -> ConnectionBootstrapTask {
        let handle = tokio::spawn(async move {
            let ConnectionBootstrap {
                children,
                children_for_log,
                deployment,
                registry,
            } = self;

            info!(children = ?children_for_log, "Connecting to children");
            let connected_clients = Self::connect_to_children(children, &deployment)
                .await
                .expect("Failed to connect to children");

            info!("Children connected");
            registry.install(connected_clients).await;
        });
        ConnectionBootstrapTask { handle }
    }

    async fn connect_to_children(
        children: Vec<ServiceName>,
        deployment: &Deployment,
    ) -> Result<HashMap<ServiceName, RpcClient>> {
        let mut clients = HashMap::new();
        for child_svc_name in children {
            let svc_info = deployment.services.get(&child_svc_name).with_context(|| {
                format!("Child service {} not found in deployment", child_svc_name)
            })?;
            let client = Self::connect_to_child(svc_info).await?;
            info!(service = %child_svc_name.as_str(), "Connected to child service");
            clients.insert(child_svc_name.clone(), client);
        }
        Ok(clients)
    }

    async fn connect_to_child(svc_info: &ServiceDiscoveryInfo) -> Result<RpcClient> {
        let ip = svc_info.ip.clone();
        let channel = LoadBalancedChannel::new(
            ip,
            svc_info.port,
            svc_info
                .replicas
                .try_into()
                .expect("Replica count too high"),
        )
        .await;

        Ok(ServiceClient::new(channel))
    }
}
