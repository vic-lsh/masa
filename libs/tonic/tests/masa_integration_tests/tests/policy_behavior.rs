#![cfg(any(
    all(feature = "sched_slo", feature = "trace_queue_latency"),
    all(feature = "sched_slo", feature = "abort_slo"),
    all(feature = "sched_slo", feature = "ac_rajomon")
))]

use std::net::{SocketAddr, TcpListener};
use std::time::Duration;

#[cfg(any(feature = "abort_slo", feature = "ac_rajomon"))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use masa::MasaRequestExt;
#[cfg(feature = "trace_queue_latency")]
use masa::MasaResponseExt;
use masa_core::{time_now, ContextBuilder};
use masa_integration_tests::pb::{
    child_service_client::ChildServiceClient,
    child_service_server::{ChildService, ChildServiceServer},
    Input1, Input2, Output1, Output2,
};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[cfg(any(feature = "abort_slo", feature = "ac_rajomon"))]
use tonic::Code;

fn unused_local_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port");
    listener.local_addr().expect("read ephemeral test port")
}

/// Install test-local Rajomon params before the process-wide `PolicyParams`
/// `OnceLock` is initialized. The production default (`init_price=0`,
/// `price_freq=5`) was chosen to match the NSDI '25 paper's "no artificial
/// floor" semantics, but that makes these admission/piggyback assertions
/// non-deterministic in a one-shot test: with `init_price=0` and no queueing,
/// `accumulated_price` stays at 0, so `tokens(0)` is admitted and the
/// propagated price is `"0"`.
///
/// We force `init_price=1` (baseline positive price so `tokens(0)` fails the
/// admission gate) and `price_freq=1` (always propagate, removing the 20%
/// retry loop). All other params take their built-in defaults via
/// `#[serde(default)]` on `PolicyParams` / `RajomonParams`.
#[cfg(feature = "ac_rajomon")]
fn ensure_test_rajomon_params() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let path =
            std::env::temp_dir().join(format!("masa_rajomon_test_{}.json", std::process::id()));
        std::fs::write(&path, r#"{"rajomon":{"init_price":1,"price_freq":1}}"#)
            .expect("write test policy params");
        std::env::set_var("MASA_POLICY_PARAMS_PATH", &path);
        // Force initialization of the OnceLock while we still hold exclusive
        // access via `Once::call_once`, so a concurrent rajomon test can't
        // read `PolicyParams::global()` before our env var is in place.
        let _ = masa_policy::PolicyParams::global();
    });
}

#[cfg(all(feature = "sched_slo", feature = "trace_queue_latency"))]
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

    let addr = unused_local_addr();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, masa_policy::PolicyHooks>::with_custom_context(QueueSvc),
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

#[cfg(all(feature = "sched_slo", feature = "abort_slo"))]
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

    let addr = unused_local_addr();
    let executed = Arc::new(AtomicBool::new(false));
    let svc = SlowSvc {
        executed: executed.clone(),
    };

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, masa_policy::PolicyHooks>::with_custom_context(svc),
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

#[cfg(all(feature = "sched_slo", feature = "ac_rajomon"))]
#[tokio::test(flavor = "current_thread")]
async fn sufficient_tokens_executes_and_piggybacks_price() {
    ensure_test_rajomon_params();
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

    let addr = unused_local_addr();
    let svc = FastSvc;

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, masa_policy::PolicyHooks>::with_custom_context(svc),
            )
            .serve_with_masa(addr)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = ChildServiceClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    // With `price_freq=1` the price is propagated on every response, so a
    // single RPC suffices. The propagated value is `accumulated_price`
    // (= `own_price + max_downstream`); with no queueing it equals the
    // configured `init_price` (1).
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

    let header = response
        .metadata()
        .get("x-masa-rajomon-price")
        .expect("price header should have been piggybacked");
    assert_eq!(header.to_str().unwrap(), "1");

    server.abort();
}

#[cfg(all(feature = "sched_slo", feature = "ac_rajomon"))]
#[tokio::test(flavor = "current_thread")]
async fn insufficient_tokens_triggers_early_return() {
    ensure_test_rajomon_params();
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

    let addr = unused_local_addr();
    let executed = Arc::new(AtomicBool::new(false));
    let svc = SlowSvc {
        executed: executed.clone(),
    };

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, masa_policy::PolicyHooks>::with_custom_context(svc),
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
    // The /EarlyReturn message should carry a structured `reason=` so the
    // experiment plotting code can attribute drops to admission control.
    assert!(
        error.message().starts_with("/EarlyReturn?src="),
        "unexpected error format: {}",
        error.message(),
    );
    assert!(
        error.message().contains("&reason=RajomonAdmissionRej"),
        "expected RajomonAdmissionRej reason, got: {}",
        error.message(),
    );
    assert!(
        !executed.load(Ordering::SeqCst),
        "handler should not have executed when token budget is exhausted"
    );

    server.abort();
}
