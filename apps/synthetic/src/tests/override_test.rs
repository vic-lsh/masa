use crate::child::server::ChildImpl;
use crate::config::{CallGraphConfig, ServiceDefinition, ServiceMethod, SyntheticConfig};
use crate::distribution::LatencyDistribution;
use crate::tonic::{child, child::child_client::ChildClient, child::child_server::ChildServer};
use app_utils::timing::time_now;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::masa::{MasaRequestExt, METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
use tonic::transport::Server;
use tonic::Request;

#[tokio::test]
async fn test_override_headers() {
    // 1. Setup a minimal ChildImpl server
    let config = SyntheticConfig {
        child_services: vec![],
        request_a_hops: vec![],
        request_b_hops: vec![],
        child_cpus_per_replica: 1.0,
        call_graph: Some(CallGraphConfig {
            entry_point: "TestService::test_method".to_string(),
            services: vec![ServiceDefinition {
                id: "TestService".to_string(),
                replicas: 1,
                methods: vec![ServiceMethod {
                    name: "test_method".to_string(),
                    latency_distribution: LatencyDistribution::Normal {
                        mean: 1000.0,
                        std: 0.0,
                        dist: rand_distr::Normal::new(1000.0, 0.0).unwrap(),
                    },
                    call_sequence_raw: vec![],
                    parsed_call_sequence: vec![],
                    busy_spin_ratio: None,
                }],
            }],
        }),
    };

    // We need to set the environment variable for SERVICE_ID as ChildImpl expects it
    std::env::set_var("SERVICE_ID", "TestService");

    let service = ChildImpl::new(config).await;

    // 2. Start the server on a random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ChildServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Create a client and send a request with overrides
    let mut client = ChildClient::connect(format!("http://{}", addr))
        .await
        .unwrap();

    let sent_at = time_now();
    let mut request = Request::new(child::MethodRequest {
        service_id: "TestService".to_string(),
        method_name: "test_method".to_string(),
        sent_at,
    });

    // Set overrides
    request
        .set_service_name_override("OverriddenService")
        .unwrap();
    request
        .set_method_name_override("overridden_method")
        .unwrap();

    // Verify headers are set correctly in the request
    let metadata = request.metadata();
    assert_eq!(
        metadata.get(SERVICE_NAME_OVERRIDE_HEADER).unwrap(),
        "OverriddenService"
    );
    assert_eq!(
        metadata.get(METHOD_NAME_OVERRIDE_HEADER).unwrap(),
        "overridden_method"
    );

    // 4. Send request and verify success
    // Note: The actual verification that the server *uses* these headers for tracing
    // would require inspecting the internal Masa tracing state or logs, which is harder
    // to do in an integration test without exposing internal state.
    // However, this test at least confirms the headers are propagated correctly via the trait.

    let response = client.handle_method(request).await;
    assert!(response.is_ok(), "Request failed: {:?}", response.err());

    // Clean up
    server_handle.abort();
}
