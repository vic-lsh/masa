use socialnet::user_mention::server::create_service;
use std::env;
use std::net::SocketAddr;
use tonic::transport::Server;

#[cfg(feature = "sched_mt")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app_utils::runtime::block_on(main_inner())
}

#[cfg(not(feature = "sched_mt"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    main_inner().await
}

async fn main_inner() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr =
        env::var("USER_MENTION_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let addr = listen_addr.parse::<SocketAddr>()?;

    // Create the service
    let service = create_service().await;

    println!("User Mention Service listening on {}", addr);

    // Run the server
    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
