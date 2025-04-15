use std::net::SocketAddr;
use tonic::transport::Server;

use socialnet::url_shorten::url_shorten_service_client::UrlShortenServiceClient;
use socialnet::url_shorten::{ComposeUrlsRequest, GetExtendedUrlsRequest};
use socialnet::url_shorten::server::create_service;

async fn setup_test_server(port: u16) -> (tokio::task::JoinHandle<()>, UrlShortenServiceClient<tonic::transport::Channel>) {
    let server_addr = format!("[::1]:{}", port);
    let server_addr_socket: SocketAddr = server_addr.parse().expect("Invalid address");
    
    println!("Starting test server on {}", server_addr);
    
    let service = create_service().await.expect("Failed to create service");
    
    // Spawn the server in a background task
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve(server_addr_socket)
            .await
            .expect("Server failed to start");
    });
    
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    

    let client_addr = format!("http://{}", server_addr);
    let client = UrlShortenServiceClient::connect(client_addr)
        .await
        .expect("Failed to connect to server");
    
    println!("Test client connected to server");
    
    (server_handle, client)
}

#[tokio::test]
async fn test_compose_urls() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50054).await;
    
    // Test case: Basic URL shortening
    let request = tonic::Request::new(ComposeUrlsRequest {
        req_id: 1,
        urls: vec!["https://www.rust-lang.org".to_string(), "https://github.com".to_string()],
    });
    
    let response = client.compose_urls(request).await?;
    let result = response.into_inner();
    
    assert!(result.exception.is_none(), 
        "Unexpected exception: {:?}", result.exception);
    assert_eq!(result.urls.len(), 2, 
        "Expected 2 URLs, got {}", result.urls.len());
    
    // Verify the URLs have the expected format
    for url in &result.urls {
        assert!(url.shortened_url.starts_with("http://short-url/"), 
            "Shortened URL has unexpected format: {}", url.shortened_url);
        assert_eq!(url.shortened_url.len(), "http://short-url/".len() + 10,
            "Shortened URL has unexpected length: {}", url.shortened_url);
    }
    
    server_handle.abort();
    
    Ok(())
}

#[tokio::test]
async fn test_get_extended_urls() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50055).await;
    
    let compose_request = tonic::Request::new(ComposeUrlsRequest {
        req_id: 1,
        urls: vec!["https://www.rust-lang.org".to_string()],
    });
    
    let compose_response = client.compose_urls(compose_request).await?;
    let compose_result = compose_response.into_inner();
    
    let shortened_url = &compose_result.urls[0].shortened_url;
    let original_url = &compose_result.urls[0].expanded_url;
    
    // Now try to get the extended URL
    let get_request = tonic::Request::new(GetExtendedUrlsRequest {
        req_id: 2,
        shortened_urls: vec![shortened_url.clone()],
    });
    
    let get_response = client.get_extended_urls(get_request).await?;
    let get_result = get_response.into_inner();
    
    assert!(get_result.exception.is_none(), 
        "Unexpected exception: {:?}", get_result.exception);
    assert_eq!(get_result.expanded_urls.len(), 1, 
        "Expected 1 URL, got {}", get_result.expanded_urls.len());
    assert_eq!(get_result.expanded_urls[0], *original_url, 
        "Got unexpected expanded URL: {}", get_result.expanded_urls[0]);
    
    server_handle.abort();
    
    Ok(())
}

#[tokio::test]
async fn test_nonexistent_url() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50056).await;
    
    let get_request = tonic::Request::new(GetExtendedUrlsRequest {
        req_id: 1,
        shortened_urls: vec!["http://short-url/nonexistent".to_string()],
    });
    
    let get_response = client.get_extended_urls(get_request).await?;
    let get_result = get_response.into_inner();
    
    assert!(get_result.exception.is_some(), 
        "Expected an exception for nonexistent URL");
    
    if let Some(exception) = get_result.exception {
        assert!(exception.message.contains("not found"), 
            "Expected error about URL not found, got: {}", exception.message);
    }
    
    server_handle.abort();
    
    Ok(())
}

#[tokio::test]
async fn test_empty_input() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50057).await;
    
    let compose_request = tonic::Request::new(ComposeUrlsRequest {
        req_id: 1,
        urls: vec![],
    });
    
    let compose_response = client.compose_urls(compose_request).await?;
    let compose_result = compose_response.into_inner();
    
    assert!(compose_result.exception.is_none(), 
        "Unexpected exception for empty input: {:?}", compose_result.exception);
    assert_eq!(compose_result.urls.len(), 0, 
        "Expected empty result, got {} URLs", compose_result.urls.len());

    let get_request = tonic::Request::new(GetExtendedUrlsRequest {
        req_id: 2,
        shortened_urls: vec![],
    });
    
    let get_response = client.get_extended_urls(get_request).await?;
    let get_result = get_response.into_inner();
    
    assert!(get_result.exception.is_none(), 
        "Unexpected exception for empty input: {:?}", get_result.exception);
    assert_eq!(get_result.expanded_urls.len(), 0, 
        "Expected empty result, got {} URLs", get_result.expanded_urls.len());
    
    server_handle.abort();
    
    Ok(())
}

#[tokio::test]
async fn test_full_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let (server_handle, mut client) = setup_test_server(50058).await;
    
    // 1. Create multiple URLs
    let urls = vec![
        "https://www.rust-lang.org".to_string(),
        "https://github.com".to_string(),
        "https://www.example.com".to_string(),
    ];
    
    let compose_request = tonic::Request::new(ComposeUrlsRequest {
        req_id: 1,
        urls: urls.clone(),
    });
    
    let compose_response = client.compose_urls(compose_request).await?;
    let compose_result = compose_response.into_inner();
    
    assert_eq!(compose_result.urls.len(), 3, "Expected 3 shortened URLs");
    
    // 2. Get all the shortened URLs
    let shortened_urls: Vec<String> = compose_result.urls
        .iter()
        .map(|url| url.shortened_url.clone())
        .collect();
    
    // 3. Retrieve the original URLs
    let get_request = tonic::Request::new(GetExtendedUrlsRequest {
        req_id: 2,
        shortened_urls,
    });
    
    let get_response = client.get_extended_urls(get_request).await?;
    let get_result = get_response.into_inner();
    
    // 4. Verify we got all the original URLs back in the correct order
    assert_eq!(get_result.expanded_urls.len(), 3, "Expected 3 expanded URLs");
    
    for (i, url) in get_result.expanded_urls.iter().enumerate() {
        assert_eq!(url, &urls[i], 
            "URL at position {} doesn't match: expected {}, got {}", 
            i, urls[i], url);
    }

    server_handle.abort();
    
    Ok(())
}