use hdrhistogram::Histogram;
use masa::Context;
use std::time::{Duration, Instant};
use tonic::metadata::MetadataMap;
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

fn benchmark_serialization() {
    println!("--- Microbenchmark: Serialization ---");
    let ctx = masa::create_context("test-api", Duration::from_micros(100));

    let iterations = 100_000;

    // Measure to_header_string
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = ctx.to_header_string();
    }
    let elapsed = start.elapsed();
    println!("to_header_string: {:?} per op", elapsed / iterations);

    let b64 = ctx.to_header_string();

    // Measure from_header_string
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = Context::from_header_string(&b64);
    }
    let elapsed = start.elapsed();
    println!("from_header_string: {:?} per op", elapsed / iterations);

    // Measure MetadataMap insertion
    let mut map = MetadataMap::new();
    let start = Instant::now();
    for _ in 0..iterations {
        map.insert_ctx("x-masa-context", &ctx);
        map.remove("x-masa-context"); // cleanup to keep map size constant
    }
    let elapsed = start.elapsed();
    println!("MetadataMap::insert_ctx: {:?} per op", elapsed / iterations);

    // Measure MetadataMap extraction
    map.insert_ctx("x-masa-context", &ctx);
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = map.get_ctx("x-masa-context");
    }
    let elapsed = start.elapsed();
    println!("MetadataMap::get_ctx: {:?} per op", elapsed / iterations);
}

async fn benchmark_e2e() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n--- E2E Benchmark: Ping Latency ---");
    let addr = "[::1]:50051".parse()?;
    let frontend = MyFrontend::default();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Start server and keep an owned handle for lifecycle management.
    let server_task = tokio::spawn(async move {
        Server::builder()
            .add_service(FrontendServer::new(frontend))
            .serve_with_shutdown(addr, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut client =
        frontend::frontend_client::FrontendClient::connect("http://[::1]:50051").await?;

    let iterations = 10_000;
    let mut hist = Histogram::<u64>::new(3).unwrap();
    let ctx = masa::create_context("test-api", Duration::from_micros(100));

    for _ in 0..iterations {
        let mut req = Request::new(PingRequest {
            message: "ping".into(),
        });

        // Attach Masa context
        req.metadata_mut().insert_ctx("x-masa-context", &ctx);

        let start = Instant::now();
        let _ = client.handle_ping(req).await?;
        let elapsed = start.elapsed().as_micros() as u64;
        hist.record(elapsed)?;
    }

    println!("E2E Latency (micros):");
    println!("  Min: {}", hist.min());
    println!("  P50: {}", hist.value_at_quantile(0.5));
    println!("  P95: {}", hist.value_at_quantile(0.95));
    println!("  P99: {}", hist.value_at_quantile(0.99));
    println!("  Max: {}", hist.max());

    let _ = shutdown_tx.send(());
    let _ = server_task.await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    benchmark_serialization();
    benchmark_e2e().await?;
    Ok(())
}
