use std::task::{Context, Poll};

use tonic::body::BoxBody;
use tonic::http;
use tonic::transport::Endpoint;
use tonic::{client::GrpcService, transport::masa_channel::Channel};
use tower::Service;

#[derive(Clone)]
pub struct LoadBalancedChannel {
    channel: Channel,
}

impl LoadBalancedChannel {
    pub async fn new_from(hostname_base: String, port: u16, replicas: u8, start: u8) -> Self {
        let mut endpoints = Vec::new();
        for i in 0..replicas {
            let endpoint =
                Endpoint::from_shared(format!("http://{}-{}:{}", hostname_base, i + start, port))
                    .unwrap();
            endpoints.push(endpoint);
        }

        let channel = Channel::new(endpoints.into_iter()).await;

        Self { channel }
    }

    pub async fn new(hostname_base: String, port: u16, replicas: u8) -> Self {
        Self::new_from(hostname_base, port, replicas, 1).await
    }
}

impl Service<http::Request<BoxBody>> for LoadBalancedChannel {
    type Response = http::Response<<Channel as GrpcService<BoxBody>>::ResponseBody>;
    type Error = <Channel as GrpcService<BoxBody>>::Error;
    type Future = <Channel as GrpcService<BoxBody>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        GrpcService::poll_ready(&mut self.channel, cx)
    }

    fn call(&mut self, request: http::Request<BoxBody>) -> Self::Future {
        GrpcService::call(&mut self.channel, request)
    }
}
