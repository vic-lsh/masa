//! Masa's implementation of tonic's channel.

use super::service::{Connection, SharedExec};
use crate::body::BoxBody;
use crate::transport::channel::{ResponseFuture, Svc, DEFAULT_BUFFER_SIZE};
use crate::transport::{Endpoint, Executor};
use http::Request;
use std::{
    marker::PhantomData,
    task::{Context, Poll},
};

use tower::{
    buffer::Buffer,
    util::{BoxService, Either},
    Service,
};

#[allow(missing_debug_implementations)]
struct Balance<S, Req> {
    services: Vec<S>,
    next: usize,
    _req: PhantomData<Req>,
}

impl<S, Req> Balance<S, Req> {
    fn new(list: impl Iterator<Item = S>) -> Self {
        Self {
            services: list.collect(),
            next: 0,
            _req: PhantomData,
        }
    }
}

impl<S, Req> Service<Req> for Balance<S, Req>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.services[self.next].poll_ready(cx)
    }

    fn call(&mut self, request: Req) -> Self::Future {
        let response = self.services[self.next].call(request);
        self.next = (self.next + 1) % self.services.len();
        response
    }
}

/// minimal reimplementation of crate::transport::channel::Channel:
/// - uses a fixed list of services for load balancing and eagerly connects to them
/// - uses fixed-list round-robin load balancing
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
        SharedExec::tokio().execute(Box::pin(worker));

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
