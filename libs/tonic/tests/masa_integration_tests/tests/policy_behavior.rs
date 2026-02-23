#![cfg(any(
    all(feature = "prio_global", feature = "trace-queue"),
    all(feature = "prio_global", feature = "early"),
    all(feature = "prio_global", feature = "rajomon")
))]

use std::time::Duration;

#[cfg(any(feature = "early", feature = "rajomon"))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use masa_core::{time_now, ContextBuilder};
use masa_integration_tests::pb::{
    child_service_client::ChildServiceClient,
    child_service_server::{ChildService, ChildServiceServer},
    Input1, Input2, Output1, Output2,
};
use tonic::masa::context::MasaRequestExt;
#[cfg(feature = "trace-queue")]
use tonic::masa::context::MasaResponseExt;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[cfg(any(feature = "early", feature = "rajomon"))]
use tonic::Code;

#[cfg(all(feature = "prio_global", feature = "trace-queue"))]
#[tokio::test(flavor = "current_thread")]
async fn queue_latency_metadata_is_attached() {
    struct QueueSvc;

    #[tonic::async_trait]
    impl ChildService for QueueSvc {
        async fn rpc1(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
            Ok(Response::new(Output1 {}))
        }

        async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
            Ok(Response::new(Output2 {}))
        }
    }

    let addr = "127.0.0.1:60070".parse().unwrap();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, tonic::masa::DefaultMasaHooks>::with_custom_context(
                    QueueSvc,
                ),
            )
            .serve_with_masa(addr)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = ChildServiceClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    let now = time_now();
    let ctx = ContextBuilder::new("test.ChildService/Rpc1", 1)
        .gateway_entry(now)
        .slo(1_000_000)
        .deadline(now + 1_000_000)
        .build();

    let mut request = Request::new(Input1 {});
    request.set_masa_context(&ctx);

    let response = client.rpc1(request).await.unwrap();
    let masa_ctx = response
        .get_masa_context()
        .expect("response missing masa context");
    let queue_latencies = masa_ctx
        .queue_latencies
        .expect("queue latency metadata not injected");
    assert!(queue_latencies.initial < 5_000_000);
    assert!(queue_latencies.resume < 5_000_000);

    server.abort();
}

#[cfg(all(feature = "prio_global", feature = "early"))]
#[tokio::test(flavor = "current_thread")]
async fn expired_context_triggers_early_return() {
    #[derive(Clone)]
    struct SlowSvc {
        executed: Arc<AtomicBool>,
    }

    #[tonic::async_trait]
    impl ChildService for SlowSvc {
        async fn rpc1(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
            self.executed.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Response::new(Output1 {}))
        }

        async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
            Ok(Response::new(Output2 {}))
        }
    }

    let addr = "127.0.0.1:60071".parse().unwrap();
    let executed = Arc::new(AtomicBool::new(false));
    let svc = SlowSvc {
        executed: executed.clone(),
    };

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, tonic::masa::DefaultMasaHooks>::with_custom_context(svc),
            )
            .serve_with_masa(addr)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = ChildServiceClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    let now = time_now();
    let expired_ctx = ContextBuilder::new("test.ChildService/Rpc1", 99)
        .gateway_entry(now.saturating_sub(2_000_000))
        .slo(1_000)
        .deadline(now.saturating_sub(1))
        .build();

    let mut request = Request::new(Input1 {});
    request.set_masa_context(&expired_ctx);

    let error = client
        .rpc1(request)
        .await
        .expect_err("request should have failed");
    assert_eq!(error.code(), Code::DeadlineExceeded);
    assert!(
        !executed.load(Ordering::SeqCst),
        "handler should not have executed when early return fires"
    );

    server.abort();
}

#[cfg(all(feature = "prio_global", feature = "rajomon"))]
#[tokio::test(flavor = "current_thread")]
async fn sufficient_tokens_executes_and_piggybacks_price() {
    #[derive(Clone)]
    struct FastSvc;

    #[tonic::async_trait]
    impl ChildService for FastSvc {
        async fn rpc1(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
            Ok(Response::new(Output1 {}))
        }

        async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
            Ok(Response::new(Output2 {}))
        }
    }

    let addr = "127.0.0.1:60073".parse().unwrap();
    let svc = FastSvc;

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, tonic::masa::DefaultMasaHooks>::with_custom_context(svc),
            )
            .serve_with_masa(addr)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = ChildServiceClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    let now = time_now();
    let rajomon_ctx = ContextBuilder::new("test.ChildService/Rpc1", 99)
        .gateway_entry(now)
        .slo(1_000_000)
        .deadline(now + 1_000_000)
        .tokens(1_000_000) // Plenty of tokens
        .build();

    let mut request = Request::new(Input1 {});
    request.set_masa_context(&rajomon_ctx);

    let response = client
        .rpc1(request)
        .await
        .expect("request should have succeeded");

    // Check that piggybacked price is 1
    let price_header = response
        .metadata()
        .get("x-masa-rajomon-price")
        .expect("missing piggybacked price");
    assert_eq!(price_header.to_str().unwrap(), "1");

    server.abort();
}

#[cfg(all(feature = "prio_global", feature = "rajomon"))]
#[tokio::test(flavor = "current_thread")]
async fn insufficient_tokens_triggers_early_return() {
    #[derive(Clone)]
    struct SlowSvc {
        executed: Arc<AtomicBool>,
    }

    #[tonic::async_trait]
    impl ChildService for SlowSvc {
        async fn rpc1(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
            self.executed.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Response::new(Output1 {}))
        }

        async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
            Ok(Response::new(Output2 {}))
        }
    }

    let addr = "127.0.0.1:60072".parse().unwrap();
    let executed = Arc::new(AtomicBool::new(false));
    let svc = SlowSvc {
        executed: executed.clone(),
    };

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, tonic::masa::DefaultMasaHooks>::with_custom_context(svc),
            )
            .serve_with_masa(addr)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = ChildServiceClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    let now = time_now();
    // Start with very few tokens
    let rajomon_ctx = ContextBuilder::new("test.ChildService/Rpc1", 99)
        .gateway_entry(now)
        .slo(1_000_000)
        .deadline(now + 1_000_000)
        .tokens(0) // Not enough tokens to even afford baseline cost of 1
        .build();

    let mut request = Request::new(Input1 {});
    request.set_masa_context(&rajomon_ctx);

    let error = client
        .rpc1(request)
        .await
        .expect_err("request should have failed due to insufficient rajomon budget");

    assert_eq!(error.code(), Code::ResourceExhausted);
    assert!(error.message().contains("Insufficient Rajomon Tokens"));
    assert!(
        !executed.load(Ordering::SeqCst),
        "handler should not have executed when token budget is exhausted"
    );

    server.abort();
}
