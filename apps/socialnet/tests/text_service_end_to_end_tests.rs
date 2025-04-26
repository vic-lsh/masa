use std::net::SocketAddr;
use tonic::transport::Server;

use socialnet::text_service::server::create_service;
use socialnet::text_service::text_service_client::TextServiceClient;
use socialnet::text_service::TextRequest;

// Helper function to set up a test server and return a client
async fn setup_test_server(
    port: u16,
) -> (
    tokio::task::JoinHandle<()>,
    TextServiceClient<tonic::transport::Channel>,
) {
    let server_addr = format!("[::1]:{}", port);
    let server_addr_socket: SocketAddr = server_addr.parse().expect("Invalid address");

    println!("Starting test server on {}", server_addr);

    let service = create_service().await;

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
    let client = TextServiceClient::connect(client_addr)
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
    let request = tonic::Request::new(TextRequest {
        text: "@john @bob Hello! Here is the link, https://openai.com".to_string(),
    });

    let Ok(response) = client.compose_text(request).await else {
        panic!("Failed to get a response from the server");
    };
    let result = response.into_inner();

    assert!(
        result.user_mentions.len() == 2,
        "Expected 2 user mentions, got {}",
        result.user_mentions.len()
    );

    assert_eq!(
        result.user_mentions[0], "3",
        "Expected user_id: 3, got {}",
        result.user_mentions[0]
    );

    assert_eq!(
        result.user_mentions[1], "5",
        "Expected user_id: 5, got {}",
        result.user_mentions[1]
    );

    server_handle.abort();

    Ok(())
}

#[ignore]
#[tokio::test]
async fn test_empty_result() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50057).await;

    let request = tonic::Request::new(TextRequest {
        text: "".to_string(),
    });

    let Ok(response) = client.compose_text(request).await else {
        panic!("Failed to get a response from the server");
    };
    let result = response.into_inner();

    assert!(
        result.user_mentions.len() == 0,
        "Expected 0 user mentions, got {}",
        result.user_mentions.len()
    );

    assert!(
        result.urls.len() == 0,
        "Expected 0 shortened, got {}",
        result.urls.len()
    );

    assert_eq!(
        result.updated_text, "",
        "Expected empty updated text, got {}",
        result.updated_text
    );

    server_handle.abort();

    Ok(())
}
