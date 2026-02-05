use std::time::Duration;

#[cfg(feature = "prio_global_early")]
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
#[cfg(feature = "prio_global_trace")]
use tonic::masa::context::MasaResponseExt;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[cfg(feature = "prio_global_early")]
use tonic::Code;

#[cfg(feature = "prio_global_trace")]
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

#[cfg(feature = "prio_global_early")]
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
