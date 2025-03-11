use crate::media::media_service_client::MediaServiceClient;
use crate::media::ComposeMediaRequest;
use crate::server::create_service;
use std::net::SocketAddr;
use tonic::transport::Server;
mod server;

pub mod media {
    tonic::include_proto!("media");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use cargo run --bin media_server -- test for end-to-end testing
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "test" {
        run_e2e_test().await?;
        return Ok(());
    }

    // Default Server when non testing
    let addr = "[::1]:50052".parse::<SocketAddr>().unwrap();
    let service = create_service();

    println!("Media Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}

// End-to-end testing function
async fn run_e2e_test() -> Result<(), Box<dyn std::error::Error>> {
    let server_port = 50055;
    let server_addr = format!("[::1]:{}", server_port);
    let server_addr_socket: SocketAddr = server_addr.parse()?;

    println!("Starting End-to-End test with server on {}", server_addr);

    let service = create_service();

    // Spawn the server
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve(server_addr_socket)
            .await
            .expect("Server failed to start");
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Connect client to the server
    let client_addr = format!("http://{}", server_addr);
    let mut client = MediaServiceClient::connect(client_addr).await?;
    println!("Client connected to server");

    // TEST CASES

    // Test case 1: Basic successful request
    println!("\nTest case 1: Basic successful request");
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 1,
        media_types: vec!["photo".to_string(), "video".to_string()],
        media_ids: vec![101, 102],
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    if result.exception.is_some() {
        println!(
            "Test case 1 failed: Unexpected exception: {:?}",
            result.exception
        );
        return Err("Test case 1 failed".into());
    }

    if result.media.len() != 2 {
        println!(
            "Test case 1 failed: Expected 2 media items, got {}",
            result.media.len()
        );
        return Err("Test case 1 failed".into());
    }

    println!("Test case 1 passed");

    // Test case 2: Validation error
    println!("\nTest case 2: Validation error (mismatched lengths)");
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 2,
        media_types: vec!["photo".to_string(), "video".to_string()],
        media_ids: vec![201], // Only one ID for two types
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    if result.exception.is_none() {
        println!("Test case 2 failed: Expected validation exception, got none");
        return Err("Test case 2 failed".into());
    }

    println!("Test case 2 passed");

    // Test case 3: Empty lists
    println!("\nTest case 3: Empty lists");
    let request = tonic::Request::new(ComposeMediaRequest {
        req_id: 3,
        media_types: vec![],
        media_ids: vec![],
    });

    let response = client.compose_media(request).await?;
    let result = response.into_inner();

    if result.exception.is_some() {
        println!(
            "Test case 3 failed: Unexpected exception: {:?}",
            result.exception
        );
        return Err("Test case 3 failed".into());
    }

    if !result.media.is_empty() {
        println!(
            "Test case 3 failed: Expected empty media list, got {} items",
            result.media.len()
        );
        return Err("Test case 3 failed".into());
    }

    println!("Test case 3 passed");

    println!("\nAll E2E tests passed successfully!");

    // End the server task
    server_handle.abort();

    Ok(())
}
