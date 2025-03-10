use tonic::transport::Server;
use std::net::SocketAddr;
mod server;

pub mod media {
  tonic::include_proto!("media");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Define address to listen on
    let addr = "[::1]:50052".parse::<SocketAddr>().unwrap();
    
    // Create the service
    let service = server::create_service();
    
    println!("Media Service listening on {}", addr);
    
    // Run the server
    Server::builder()
        .add_service(service)
        .serve(addr)
        .await?;
    
    Ok(())
}