use crate::RpcClient;
use sim_config::svc::ServiceName;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{RwLock, RwLockReadGuard};

/// Owns the child RPC clients map together with the "bootstrap finished" flag.
///
/// The invariant `ready == true => map populated` is load-bearing: if fanout
/// silently skips a missing child, the service returns 200 OK with no
/// downstream work, producing meaningless goodput. Keeping both fields behind
/// one type ensures the flag can only flip after the map is installed, and
/// concentrates the "startup race vs. misconfig" decision in `get_expect_ready`.
pub(crate) struct ClientRegistry {
    clients: RwLock<HashMap<ServiceName, RpcClient>>,
    ready: AtomicBool,
}

pub(crate) enum ClientLookup {
    Found(RpcClient),
    /// Bootstrap hasn't finished yet — caller should warn and continue;
    /// the load-gen warmup absorbs these startup-race misses.
    StartupRace,
}

impl ClientRegistry {
    pub(crate) fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            ready: AtomicBool::new(false),
        }
    }

    /// Install the fully-connected client map and mark the registry ready.
    /// Must only be called once, from bootstrap.
    pub(crate) async fn install(&self, new_clients: HashMap<ServiceName, RpcClient>) {
        {
            let mut guard = self.clients.write().await;
            *guard = new_clients;
        }
        self.ready.store(true, Ordering::Release);
    }

    /// Force-mark ready without installing. Used by the bootstrap timeout
    /// path in `main`: if bootstrap is genuinely stuck, we'd rather panic on
    /// the first fanout than silently serve no-op traffic.
    pub(crate) fn force_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Look up a child client, distinguishing startup race from misconfig.
    /// Panics if the child is missing *after* bootstrap has signalled ready —
    /// that means `deployment.json` points at a name the registry never
    /// received, which would otherwise silently produce 100% bogus goodput.
    pub(crate) async fn get_expect_ready(&self, name: &ServiceName) -> ClientLookup {
        let guard = self.clients.read().await;
        if let Some(c) = guard.get(name) {
            return ClientLookup::Found(c.clone());
        }
        if self.ready.load(Ordering::Acquire) {
            panic!(
                "Child service {} not found in clients map after bootstrap — \
                 check deployment.json ip fields match actual service DNS names",
                name.as_str()
            );
        }
        ClientLookup::StartupRace
    }

    /// Read guard for replay callers that iterate the whole map.
    pub(crate) async fn read(&self) -> RwLockReadGuard<'_, HashMap<ServiceName, RpcClient>> {
        self.clients.read().await
    }

    #[cfg(test)]
    pub(crate) async fn insert_for_test(&self, svc: ServiceName, client: RpcClient) {
        self.clients.write().await.insert(svc, client);
    }
}
