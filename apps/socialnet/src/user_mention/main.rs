use socialnet::user_mention::server::create_service;
use std::net::SocketAddr;
use std::env;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let addr = "[::1]:50052".parse::<SocketAddr>().unwrap();

    // // Create the service
    // let service = create_service().await;

    // println!("User Mention Service listening on {}", addr);

    // // Run the server
    // Server::builder().add_service(service).serve(addr).await?;

    // Ok(())
    let listen_addr = env::var("USER_MENTION_LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    
    let addr = listen_addr.parse::<SocketAddr>()?;

    // Create the service
    let service = create_service().await;

    println!("User Mention Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
