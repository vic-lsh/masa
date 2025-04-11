use crate::server::create_service;
use std::net::SocketAddr;
use tonic::transport::Server;
mod server;

pub mod media {
    tonic::include_proto!("media");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse::<SocketAddr>().unwrap();

    // Create the service
    let service = create_service();

    println!("Media Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
