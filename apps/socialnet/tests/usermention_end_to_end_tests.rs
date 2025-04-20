use std::net::SocketAddr;
use tonic::transport::Server;

use socialnet::user_mention::user_mention_service_client::UserMentionServiceClient;
use socialnet::user_mention::server::create_service;
use socialnet::user_mention::ComposeUserMentionRequest;

// Helper function to set up a test server and return a client
async fn setup_test_server(
    port: u16,
) -> (
    tokio::task::JoinHandle<()>,
    UserMentionServiceClient<tonic::transport::Channel>,
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
    let client = UserMentionServiceClient::connect(client_addr)
        .await
        .expect("Failed to connect to server");

    println!("Test client connected to server");

    (server_handle, client)
}

#[tokio::test]
async fn test_basic_successful_request() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50056).await;

    // Test case: Basic successful request
    let request = tonic::Request::new(ComposeUserMentionRequest {
        req_id: 1,
        usernames: vec!["adam".to_string(), "alice".to_string()],
    });

    let response = client.compose_user_mentions(request).await?;
    let result = response.into_inner();

    assert!(
        result.exception.is_none(),
        "Unexpected exception: {:?}",
        result.exception
    );


    assert!(
        result.user_mentions.len() == 2,
        "Expected 2 user mentions, got {}",
        result.user_mentions.len()
    );

    assert_eq!(
        result.user_mentions[0].username, "adam@memcached",
        "Expected adam@memcached, got {}", result.user_mentions[0].username
    );

    assert_eq!(
        result.user_mentions[1].username, "alice@mongodb",
        "Expected alice@mongodb, got {}", result.user_mentions[1].username
    );

    server_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_empty_result() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50057).await;

    let request = tonic::Request::new(ComposeUserMentionRequest {
        req_id: 1,
        usernames: vec!["baris".to_string(), "arvind".to_string()],
    });

    let response = client.compose_user_mentions(request).await?;
    let result = response.into_inner();

    assert!(
        result.exception.is_none(),
        "Unexpected exception: {:?}",
        result.exception
    );


    assert!(
        result.user_mentions.len() == 0,
        "Expected 0 user mentions, got {}",
        result.user_mentions.len()
    );

    server_handle.abort();

    Ok(())
}

