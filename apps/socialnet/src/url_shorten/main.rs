use socialnet::url_shorten::server::create_service;
use std::net::SocketAddr;
use tonic::transport::Server;

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50053".parse::<SocketAddr>().unwrap();

    let service = create_service().await?;
    
    println!("URL Shortening Service listening on {}", addr);
    
    Server::builder()
        .add_service(service)
        .serve(addr)
        .await?;
    
    Ok(())
}