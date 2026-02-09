use crate::service_stubs::service_client::ServiceClient;
use crate::RpcClient;
use anyhow::{Context, Result};
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::ServiceName;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tracing::info;

pub(crate) struct ConnectionBootstrap {
    children: Vec<ServiceName>,
    children_for_log: Vec<(ServiceName, u64)>,
    deployment: Deployment,
    clients: Arc<RwLock<HashMap<ServiceName, RpcClient>>>,
}

pub(crate) struct ConnectionBootstrapTask {
    handle: JoinHandle<()>,
}

impl Drop for ConnectionBootstrapTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl ConnectionBootstrap {
    pub(crate) fn new(
        children: Vec<ServiceName>,
        children_for_log: Vec<(ServiceName, u64)>,
        deployment: Deployment,
        clients: Arc<RwLock<HashMap<ServiceName, RpcClient>>>,
    ) -> Self {
        Self {
            children,
            children_for_log,
            deployment,
            clients,
        }
    }

    pub(crate) fn spawn(self) -> ConnectionBootstrapTask {
        let handle = tokio::spawn(async move {
            let ConnectionBootstrap {
                children,
                children_for_log,
                deployment,
                clients,
            } = self;

            info!(children = ?children_for_log, "Connecting to children");
            let connected_clients = Self::connect_to_children(children, &deployment)
                .await
                .expect("Failed to connect to children");

            info!("Children connected");

            let mut guard = clients.write().await;
            let _ = std::mem::replace(&mut *guard, connected_clients);
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
