use socialnet::text_service::server::create_service;
use std::net::SocketAddr;
use tonic::transport::Server;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50053".parse::<SocketAddr>().unwrap();

    // Create the service
    let service = create_service().await;

    println!("Text Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
