use std::task::{Context, Poll};

use tonic::{
    body::BoxBody,
    client::GrpcService,
    http,
    transport::{Channel, Endpoint},
};
use tower::Service;

#[derive(Debug, Clone)]
pub struct LoadBalancedChannel {
    channels: Vec<Channel>,
    next: usize,
}

impl LoadBalancedChannel {
    pub async fn new(hostname_base: String, port: u16, replicas: u8) -> Self {
        let mut channels = Vec::new();

        for i in 1..=replicas {
            channels.push(
                Endpoint::from_shared(format!("http://{}-{}:{}", hostname_base, i, port))
                    .unwrap()
                    .connect()
                    .await
                    .unwrap(),
            )
        }

        Self { channels, next: 0 }
    }

    fn update_next(&mut self) {
        self.next = (self.next + 1) % self.channels.len();
    }
}

impl Service<http::Request<BoxBody>> for LoadBalancedChannel {
    type Response = http::Response<<Channel as GrpcService<BoxBody>>::ResponseBody>;
    type Error = <Channel as GrpcService<BoxBody>>::Error;
    type Future = <Channel as GrpcService<BoxBody>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        GrpcService::poll_ready(&mut self.channels[self.next], cx)
    }

    fn call(&mut self, request: http::Request<BoxBody>) -> Self::Future {
        let r = GrpcService::call(&mut self.channels[self.next], request);
        self.update_next();
        r
    }
}
