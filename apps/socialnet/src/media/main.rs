use crate::server::create_service;
use std::env;
use std::net::SocketAddr;
use tonic::transport::Server;
mod server;

pub mod media {
    tonic::include_proto!("media");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let addr = "[::1]:50051".parse::<SocketAddr>().unwrap();

    // // Create the service
    // let service = create_service();

    // println!("Media Service listening on {}", addr);

    // // Run the server
    // Server::builder().add_service(service).serve(addr).await?;

    // Ok(())
    let listen_addr =
        env::var("MEDIA_SERVICE_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let addr = listen_addr.parse::<SocketAddr>()?;

    // Create the service
    let service = create_service();

    println!("Media Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
