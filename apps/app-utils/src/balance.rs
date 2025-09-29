use anyhow::Result;
use futures::{Stream, StreamExt, TryStreamExt};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::{
    api::ListParams,
    runtime::{watcher, WatchStreamExt},
    Api, Client as K8sClient,
};
use rustls::crypto::ring;
use rustls::crypto::CryptoProvider;
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    select,
    sync::{watch, RwLock},
};
use tokio_stream::wrappers::WatchStream;
use tonic::transport::Uri;
use tonic::{
    transport::{Channel, Endpoint},
    Request,
};
use tower::{balance::p2c::Balance, discover::Change, Service};
use tracing::{error, info, warn};

fn install_rustls_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        CryptoProvider::install_default(ring::default_provider())
            .expect("install rustls ring provider");
    });
}

pub async fn spawn_endpointslice_task(
    namespace: String,
    service: String,
    named_port: Option<String>,
    tx: tokio::sync::mpsc::Sender<Change<Uri, Endpoint>>,
) -> Result<()> {
    // NOTE: this is necessary to prevent a panic when instantiating a K8sClient
    install_rustls_provider();

    tokio::spawn(async move {
        let client = K8sClient::try_default().await.expect("k8s client");
        let api: Api<EndpointSlice> = Api::namespaced(client, &namespace);

        let mut seen: HashSet<SocketAddr> = HashSet::new();
        let cfg =
            watcher::Config::default().labels(&format!("kubernetes.io/service-name={service}"));
        let mut stream = watcher(api, cfg).applied_objects().boxed();

        loop {
            select! {
                ev = stream.try_next() => match ev {
                    Ok(Some(es)) => {
                        let ports: Vec<u16> = es.ports.as_ref()
                            .map(|ps| ps.iter().filter_map(|p| {
                                if let Some(name) = &named_port {
                                    match (&p.name, p.port) {
                                        (Some(n), Some(prt)) if n == name => Some(prt as u16),
                                        _ => None,
                                    }
                                } else {
                                    p.port.map(|prt| prt as u16)
                                }
                            }).collect())
                            .unwrap_or_default();

                        if ports.is_empty() {
                            warn!("EndpointSlice {} had no matching ports",
                                  es.metadata.name.unwrap_or_default());
                            continue;
                        }

                        let mut now: HashSet<SocketAddr> = HashSet::new();
                        for ep in es.endpoints {
                            let ready = ep.conditions.as_ref().and_then(|c| c.ready).unwrap_or(false);
                            if !ready { continue; }
                            for a in ep.addresses {
                                if let Ok(ip) = a.parse::<IpAddr>() {
                                    for p in &ports {
                                        now.insert(SocketAddr::new(ip, *p));
                                    }
                                }
                            }
                        }

                        for addr in now.difference(&seen) {
                            let uri: Uri = format!("http://{addr}").parse().unwrap();
                            let ep = Endpoint::from_shared(uri.to_string()).unwrap()
                                .tcp_nodelay(true)
                                .connect_timeout(Duration::from_secs(2));
                            info!("Inserting {:?}", ep);
                            let _ = tx.try_send(Change::Insert(uri, ep));
                        }
                        for addr in seen.difference(&now) {
                            let uri: Uri = format!("http://{addr}").parse().unwrap();
                            let _ = tx.try_send(Change::Remove(uri));
                        }
                        seen = now;
                        info!("balanced endpoints: {seen:?}");
                    }
                    Ok(None) => break, // stream ended
                    Err(e) => {
                        warn!("EndpointSlice watch error: {e:?}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    });

    Ok(())
}

pub async fn spawn_endpointslice_watcher(
    namespace: String,
    service_name: String,
) -> Result<watch::Receiver<Vec<SocketAddr>>> {
    let client = K8sClient::try_default().await?;
    let api: Api<EndpointSlice> = Api::namespaced(client, &namespace);

    let (tx, rx) = watch::channel::<Vec<SocketAddr>>(Vec::new());

    tokio::spawn(async move {
        let mut current: HashSet<SocketAddr> = HashSet::new();

        let mut stream = watcher(api, watcher::Config::default())
            .applied_objects()
            .boxed();

        loop {
            select! {
                maybe_slice = stream.try_next() => {
                    match maybe_slice {
                        Ok(Some(es)) => {
                            // Collect all "ready" addresses: Ready == true and port matching the slice
                            let mut found: HashSet<SocketAddr> = HashSet::new();

                            // Determine the port to use:
                            // EndpointSlice.ports[*].port (optional; could be None, handle carefully).
                            // Here we collect all named/unnamed ports; if multiple, we include all.
                            let ports: Vec<u16> = es.ports
                                .as_ref()
                                .map(|ports| ports.iter().filter_map(|p| p.port).map(|p| p as u16).collect())
                                .unwrap_or_default();

                            if ports.is_empty() {
                                warn!("EndpointSlice {} has no ports; skipping", es.metadata.name.unwrap_or_default());
                                continue;
                            }

                            for ep in es.endpoints {
                                // Ready check: conditions.ready == Some(true)
                                let ready = ep.conditions.as_ref().and_then(|c| c.ready).unwrap_or(false);
                                if !ready { continue; }

                                for addr_str in ep.addresses {
                                    // We’ll keep both v4 and v6 (tonic supports both); skip non-parsable
                                    if let Ok(ip) = addr_str.parse::<IpAddr>() {
                                        for port in &ports {
                                            let sock = SocketAddr::new(ip, *port);
                                            found.insert(sock);
                                        }
                                    }
                                }
                            }

                            if found != current {
                                current = found;
                                let mut v: Vec<SocketAddr> = current.iter().cloned().collect();
                                // deterministic order helps RR behavior
                                v.sort_by_key(|s| (s.ip().to_string(), s.port()));
                                if tx.send(v).is_err() {
                                    // all receivers dropped; stop
                                    return;
                                }
                            }
                        }
                        Ok(None) => {
                            // stream ended; unlikely unless permissions changed – clear set
                            let _ = tx.send(Vec::new());
                            return;
                        }
                        Err(e) => {
                            error!("EndpointSlice watch error: {e:?}");
                            // brief backoff before retry; watcher already restarts internally,
                            // but we can sleep to avoid hot loop on auth issues.
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    });

    Ok(rx)
}

/// A `Discover` that turns a `watch::Receiver<Vec<SocketAddr>>`
/// into tower `Change`s.
pub struct EndpointDiscover {
    inner: WatchStream<Vec<SocketAddr>>,
    last: Vec<SocketAddr>,
}

impl EndpointDiscover {
    fn new(rx: tokio::sync::watch::Receiver<Vec<SocketAddr>>) -> Self {
        Self {
            inner: WatchStream::new(rx),
            last: Vec::new(),
        }
    }
}

impl Stream for EndpointDiscover {
    type Item = Result<Change<SocketAddr, Channel>, ()>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(addrs)) => {
                // Diff old vs new
                for addr in addrs.iter() {
                    if !self.last.contains(addr) {
                        let uri = format!("http://{}", addr);
                        let ep = Endpoint::from_shared(uri).unwrap();
                        let ch = ep.connect_lazy();
                        return Poll::Ready(Some(Ok(Change::Insert(*addr, ch))));
                    }
                }
                for addr in self.last.iter() {
                    if !addrs.contains(addr) {
                        return Poll::Ready(Some(Ok(Change::Remove(*addr))));
                    }
                }
                self.last = addrs;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

// pub async fn make_balancer(
//     namespace: String,
//     service_name: String,
// ) -> Result<
//     Balance<
//         EndpointDiscover,
//         Request<
//             http_body::combinators::box_body::UnsyncBoxBody<tonic::codegen::Bytes, tonic::Status>,
//         >,
//     >,
// > {
//     let rx = spawn_endpointslice_watcher("ns".into(), "svc".into()).await?;
//     let discover = EndpointDiscover::new(rx);
//     let mut balancer = Balance::new(discover);
//     Ok(balancer)
// }
