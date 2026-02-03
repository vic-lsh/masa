use criterion::{criterion_group, criterion_main, Criterion};

use std::time::Duration;
use tonic::{transport::Server, Request, Response, Status};

pub mod frontend {
    tonic::include_proto!("frontend");
}

use frontend::frontend_server::{Frontend, FrontendServer};
use frontend::{ARequest, AResponse, BRequest, BResponse, PingRequest, PingResponse};

#[derive(Default)]
pub struct MyFrontend {}

#[tonic::async_trait]
impl Frontend for MyFrontend {
    async fn handle_ping(
        &self,
        request: Request<PingRequest>,
    ) -> Result<Response<PingResponse>, Status> {
        let reply = PingResponse {
            message: request.into_inner().message,
        };
        Ok(Response::new(reply))
    }

    async fn handle_a(&self, _request: Request<ARequest>) -> Result<Response<AResponse>, Status> {
        Ok(Response::new(AResponse::default()))
    }

    async fn handle_b(&self, _request: Request<BRequest>) -> Result<Response<BResponse>, Status> {
        Ok(Response::new(BResponse::default()))
    }
}

// Rewriting bench_e2e to connect once
fn bench_e2e_reused_client(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // Spawn server
    rt.spawn(async {
        let addr = "[::1]:50053".parse().unwrap();
        let frontend = MyFrontend::default();
        Server::builder()
            .add_service(FrontendServer::new(frontend))
            .serve(addr)
            .await
            .unwrap();
    });

    rt.block_on(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    let client = rt.block_on(async {
        frontend::frontend_client::FrontendClient::connect("http://[::1]:50053")
            .await
            .unwrap()
    });

    let mut group = c.benchmark_group("E2E_Persistent");

    group.bench_function("ping_rpc", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = client.clone();
            let ctx = masa::create_context("test-api", Duration::from_micros(100));
            let mut req = Request::new(PingRequest {
                message: "ping".into(),
            });
            req.metadata_mut().insert_ctx("x-masa-context", &ctx);

            client.handle_ping(req).await.unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_e2e_reused_client);
criterion_main!(benches);
