//! Masa's implementation of tonic's channel.

use super::service::{Connection, SharedExec};
use crate::body::BoxBody;
use crate::transport::channel::{ResponseFuture, Svc, DEFAULT_BUFFER_SIZE};
use crate::transport::{Endpoint, Executor};
use http::Request;
use std::task::{Context, Poll};

use tower::balance::masa_balance::Balance;
use tower::{
    buffer::Buffer,
    util::{BoxService, Either},
    Service,
};

use masa::PriorityHint;

/// minimal reimplementation of crate::transport::channel::Channel:
/// - uses a fixed list of services for load balancing and eagerly connects to them
/// - uses our custom load balancing logic in tower (the Balance struct)
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct Channel {
    svc: Buffer<Svc, Request<BoxBody>>,
}

impl Channel {
    /// Create a new Masa channel.
    pub async fn new(list: impl Iterator<Item = Endpoint>) -> Self {
        let mut connections = Vec::new();
        for endpoint in list {
            let mut http = hyper::client::connect::HttpConnector::new();
            http.set_nodelay(endpoint.tcp_nodelay);
            http.set_keepalive(endpoint.tcp_keepalive);
            http.set_connect_timeout(endpoint.connect_timeout);
            http.enforce_http(false);

            let uri = endpoint.uri.clone();
            let connection = loop {
                match Connection::connect(endpoint.connector(http.clone()), endpoint.clone()).await
                {
                    Ok(conn) => {
                        log::info!("connected to endpoint {}", uri);
                        break conn;
                    }
                    Err(e) => {
                        eprintln!("failed to connect to endpoint {}: {}. Retrying...", uri, e);
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            };

            connections.push(connection);
        }
        let svc = Balance::new(connections.into_iter());

        let svc = BoxService::new(svc);
        let (svc, worker) = Buffer::pair(Either::B(svc), DEFAULT_BUFFER_SIZE);
        SharedExec::tokio().execute(Box::pin(worker), PriorityHint::infra());

        Channel { svc }
    }
}

impl Service<http::Request<BoxBody>> for Channel {
    type Response = http::Response<super::Body>;
    type Error = super::Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::poll_ready(&mut self.svc, cx).map_err(super::Error::from_source)
    }

    fn call(&mut self, request: http::Request<BoxBody>) -> Self::Future {
        let inner = Service::call(&mut self.svc, request);

        ResponseFuture { inner }
    }
}
