//! Masa-owned transport APIs.

use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use hyper::rt::Exec;
use tokio::task::TaskPriority;
use tonic::body::BoxBody;
use tonic::client::GrpcService;
use tonic::codegen::{Body, Bytes, Service, StdError};
use tonic::http;
use tonic::transport::{server::Routes, Endpoint, Error};
use tower_layer::Layer;

fn h2_stream_priority(headers: &http::HeaderMap) -> TaskPriority {
    TaskPriority::new(crate::read_priority_from_headers(headers).value())
}

/// Masa-owned extension methods for tonic servers.
pub trait MasaServerExt<ResBody> {
    /// Consume this server and serve it with Masa's H2 stream-priority executor.
    fn serve_with_masa(
        self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
}

impl<L, ResBody> MasaServerExt<ResBody> for tonic::transport::server::Router<L>
where
    L: Layer<Routes> + Send + 'static,
    L::Service: Service<http::Request<tonic::transport::Body>, Response = http::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    <<L as Layer<Routes>>::Service as Service<http::Request<tonic::transport::Body>>>::Future:
        Send + 'static,
    <<L as Layer<Routes>>::Service as Service<http::Request<tonic::transport::Body>>>::Error:
        Into<StdError> + Send,
    ResBody: Body<Data = Bytes> + Send + 'static,
    ResBody::Error: Into<StdError>,
{
    fn serve_with_masa(
        self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        Box::pin(self.serve_with_executor(addr, Exec::masa(h2_stream_priority)))
    }
}

/// A channel that load balances across a static number of replicas.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct LoadBalancedChannel {
    channel: tonic::transport::masa_channel::Channel,
}

impl LoadBalancedChannel {
    /// Construct a new `LoadBalancedChannel` with a custom start index.
    pub async fn new_from(hostname_base: String, port: u16, replicas: u8, start: u8) -> Self {
        let mut endpoints = Vec::new();
        for i in 0..replicas {
            let endpoint =
                Endpoint::from_shared(format!("http://{}-{}:{}", hostname_base, i + start, port))
                    .unwrap();
            endpoints.push(endpoint);
        }

        let channel = tonic::transport::masa_channel::Channel::new(endpoints.into_iter()).await;

        Self { channel }
    }

    /// Construct a new `LoadBalancedChannel`.
    pub async fn new(hostname_base: String, port: u16, replicas: u8) -> Self {
        Self::new_from(hostname_base, port, replicas, 1).await
    }

    /// Construct a new `LoadBalancedChannel` using service name directly.
    ///
    /// Docker Compose handles load balancing automatically when using `scale`,
    /// so this uses the service name directly instead of individual replica
    /// hostnames.
    pub async fn new_from_service_name(service_name: String, port: u16, _replicas: u8) -> Self {
        let endpoint = Endpoint::from_shared(format!("http://{}:{}", service_name, port)).unwrap();
        let channel = tonic::transport::masa_channel::Channel::new(std::iter::once(endpoint)).await;
        Self { channel }
    }
}

impl Service<http::Request<BoxBody>> for LoadBalancedChannel {
    type Response = http::Response<
        <tonic::transport::masa_channel::Channel as GrpcService<BoxBody>>::ResponseBody,
    >;
    type Error = <tonic::transport::masa_channel::Channel as GrpcService<BoxBody>>::Error;
    type Future = <tonic::transport::masa_channel::Channel as GrpcService<BoxBody>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        GrpcService::poll_ready(&mut self.channel, cx)
    }

    fn call(&mut self, request: http::Request<BoxBody>) -> Self::Future {
        GrpcService::call(&mut self.channel, request)
    }
}
