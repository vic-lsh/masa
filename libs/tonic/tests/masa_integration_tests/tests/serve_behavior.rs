use std::sync::{Arc, Mutex};
use std::time::Duration;

use masa_core::{time_now, ContextBuilder};
use masa_integration_tests::pb::{
    child_service_client::ChildServiceClient,
    child_service_server::{ChildService, ChildServiceServer},
    Input1, Input2, Output1, Output2,
};
use tokio::sync::Barrier;
use tonic::masa::context::MasaRequestExt;
use tonic::metadata::MetadataValue;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[cfg(feature = "prio_global")]
#[tokio::test(flavor = "current_thread")]
async fn high_priority_request_preempts_under_masa() {
    #[derive(Clone)]
    struct PreemptSvc {
        barrier: Arc<Barrier>,
        order: Arc<Mutex<Vec<String>>>,
    }

    #[tonic::async_trait]
    impl ChildService for PreemptSvc {
        async fn rpc1(&self, req: Request<Input1>) -> Result<Response<Output1>, Status> {
            let label = req
                .metadata()
                .get("x-test-label")
                .and_then(|val| val.to_str().ok())
                .unwrap_or("unknown")
                .to_string();

            self.barrier.wait().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.order.lock().unwrap().push(label);
            Ok(Response::new(Output1 {}))
        }

        async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
            Ok(Response::new(Output2 {}))
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let order = Arc::new(Mutex::new(Vec::new()));

    let svc = PreemptSvc {
        barrier: barrier.clone(),
        order: order.clone(),
    };

    let addr = "127.0.0.1:60110".parse().unwrap();
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

    let mut client = ChildServiceClient::connect("http://127.0.0.1:60110")
        .await
        .unwrap();

    let now = time_now();
    let low_ctx = ContextBuilder::new("test.ChildService/Rpc1", 1)
        .gateway_entry(now)
        .slo(1_000_000)
        .deadline(now + 1_000_000)
        .build();
    let high_ctx = ContextBuilder::new("test.ChildService/Rpc1", 2)
        .gateway_entry(now)
        .slo(50_000)
        .deadline(now + 50_000)
        .build();

    let mut low_req = Request::new(Input1 {});
    low_req
        .metadata_mut()
        .insert("x-test-label", MetadataValue::from_static("low"));
    low_req.set_masa_context(&low_ctx);

    let mut high_req = Request::new(Input1 {});
    high_req
        .metadata_mut()
        .insert("x-test-label", MetadataValue::from_static("high"));
    high_req.set_masa_context(&high_ctx);

    let mut low_client = client.clone();
    let low_call = tokio::spawn(async move { low_client.rpc1(low_req).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    let high_call = tokio::spawn(async move { client.rpc1(high_req).await });

    low_call.await.unwrap().unwrap();
    high_call.await.unwrap().unwrap();

    server.abort();

    let recorded = order.lock().unwrap().clone();
    assert_eq!(recorded, vec!["high", "low"]);
}
