use socialnet::user_mention::server::create_service;
use std::env;
use std::net::SocketAddr;
use tonic::transport::Server;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr =
        env::var("USER_MENTION_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let addr = listen_addr.parse::<SocketAddr>()?;

    // Create the service
    let service = create_service().await;

    println!("User Mention Service listening on {}", addr);

    // Run the server
    Server::builder()
        .add_service(service)
        .serve_with_masa(addr)
        .await?;

    Ok(())
}
