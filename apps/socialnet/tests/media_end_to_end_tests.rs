use std::net::SocketAddr;
use tonic::transport::Server;

use socialnet::media::media_service_client::MediaServiceClient;
use socialnet::media::server::create_service;
use socialnet::media::ComposeMediaRequest;

// Helper function to set up a test server and return a client
async fn setup_test_server(
    port: u16,
) -> (
    tokio::task::JoinHandle<()>,
    MediaServiceClient<tonic::transport::Channel>,
) {
    let server_addr = format!("[::1]:{}", port);
    let server_addr_socket: SocketAddr = server_addr.parse().expect("Invalid address");

    println!("Starting test server on {}", server_addr);

    let service = create_service();

    // Spawn the server in a background task
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve(server_addr_socket)
            .await
            .expect("Server failed to start");
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Create the client
    let client_addr = format!("http://{}", server_addr);
    let client = MediaServiceClient::connect(client_addr)
        .await
        .expect("Failed to connect to server");

    println!("Test client connected to server");

    (server_handle, client)
}

#[ignore]
#[tokio::test]
async fn test_basic_successful_request() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50056).await;

    // Test case: Basic successful request
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 1,
        media_types: vec!["photo".to_string(), "video".to_string()],
        media_ids: vec![101, 102],
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    assert!(
        result.exception.is_none(),
        "Unexpected exception: {:?}",
        result.exception
    );
    assert_eq!(
        result.media.len(),
        2,
        "Expected 2 media items, got {}",
        result.media.len()
    );

    server_handle.abort();

    Ok(())
}

#[ignore]
#[tokio::test]
async fn test_validation_error() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50057).await;

    // Test case: Validation error (mismatched lengths)
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 2,
        media_types: vec!["photo".to_string(), "video".to_string()],
        media_ids: vec![201], // Only one ID for two types
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    assert!(
        result.exception.is_some(),
        "Expected validation exception, got none"
    );

    server_handle.abort();

    Ok(())
}

#[ignore]
#[tokio::test]
async fn test_empty_lists() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50058).await;

    // Test case: Empty lists
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 3,
        media_types: vec![],
        media_ids: vec![],
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    assert!(
        result.exception.is_none(),
        "Unexpected exception: {:?}",
        result.exception
    );
    assert!(
        result.media.is_empty(),
        "Expected empty media list, got {} items",
        result.media.len()
    );

    server_handle.abort();

    Ok(())
}
