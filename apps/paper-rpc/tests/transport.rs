use paper_rpc::{
    pb::{echo_client::EchoClient, Input},
    serve, EchoService,
};
use rajomon::{PaperRajomonModule, RajomonWire, CLIENT_TOKEN_BUCKET};
use rpcstack_tonic::{RequestExt, StatusExt};
use tailclipper::{TailClipperModule, TailClipperWire};
use tonic::{Code, CowGrpcMethod, Request};

#[tokio::test(flavor = "current_thread")]
async fn two_hops_propagate_metadata_and_feed_back_rejection_prices() {
    // One test in its own binary: deterministic advertised price, positive
    // own price exposes any accidental per-hop token subtraction.
    let path = std::env::temp_dir().join(format!("paper-rpc-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"rajomon":{"init_price":7,"price_freq":1,"price_update_rate_ms":60000,"token_update_rate_ms":60000}}"#).unwrap();
    std::env::set_var(rajomon::PARAMS_PATH_ENV, &path);
    let (leaf, leaf_task) = serve(EchoService::default()).await.unwrap();
    let downstream = EchoClient::connect(leaf).await.unwrap();
    let (front, front_task) = serve(EchoService {
        downstream: Some(downstream),
    })
    .await
    .unwrap();
    let mut client = EchoClient::connect(front).await.unwrap();
    let mut request = Request::new(Input {});
    request.set_wire::<TailClipperModule>(&TailClipperWire { arrival_ns: 123456 });
    request.set_wire::<PaperRajomonModule>(&RajomonWire::request(20));
    let output = client.call(request).await.unwrap().into_inner();
    assert_eq!(
        (output.hops, output.arrival_ns, output.tokens),
        (2, 123456, 20)
    );

    let mut poor = Request::new(Input {});
    poor.set_wire::<PaperRajomonModule>(&RajomonWire::request(0));
    let status = client.call(poor).await.unwrap_err();
    assert_eq!(status.code(), Code::ResourceExhausted);
    assert_eq!(
        status.get_wire::<PaperRajomonModule>().unwrap().price,
        Some(14)
    );

    // Root admission consumes no transport or handler work after learning
    // that the current balance cannot pay the cached interface price.
    CLIENT_TOKEN_BUCKET.deduct(u64::MAX);
    assert!(
        CLIENT_TOKEN_BUCKET
            .try_acquire(&CowGrpcMethod::new("paper.Echo", "Call"))
            .is_none(),
        "root did not learn price"
    );
    let status = client.call(Input {}).await.unwrap_err();
    assert_eq!(status.message(), "Rajomon root rate limit");
    assert!(CLIENT_TOKEN_BUCKET
        .try_acquire(&CowGrpcMethod::new("paper.Echo", "Call"))
        .is_none());
    front_task.abort();
    leaf_task.abort();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_shares_only_the_six_oldest_runnable_requests() {
    use std::sync::{Arc, Mutex};
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for id in 1..=8 {
        let order = order.clone();
        handles.push(tokio::task::spawn_with_prio(
            async move {
                for _ in 0..4 {
                    order.lock().unwrap().push(id);
                    tailclipper::yield_slice().await;
                }
            },
            tokio::task::TaskPriority::new(id),
        ));
    }
    for task in handles {
        task.await.unwrap();
    }
    let order = order.lock().unwrap();
    assert_eq!(&order[..24], &[1, 2, 3, 4, 5, 6].repeat(4));
    assert_eq!(&order[24..], &[7, 8].repeat(4));
}
