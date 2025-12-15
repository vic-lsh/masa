use socialnet::text_service::server::create_service;
use std::net::SocketAddr;
use tonic::transport::Server;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = env::var("TEXT_SERVICE_LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    
    let addr = listen_addr.parse::<SocketAddr>()?;

    // Create the service
    let service = create_service().await;

    println!("Text Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
