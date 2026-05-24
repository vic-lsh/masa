//! Masa-owned transport APIs.

use std::task::{Context, Poll};

use tonic::body::BoxBody;
use tonic::client::GrpcService;
use tonic::codegen::Service;
use tonic::http;
use tonic::transport::Endpoint;

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
