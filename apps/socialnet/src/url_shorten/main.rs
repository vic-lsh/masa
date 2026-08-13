use socialnet::url_shorten::server::create_service;
use std::env;
use std::net::SocketAddr;
use tonic::transport::Server;

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
}

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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    println!("inside url_shorten");

    let listen_addr =
        env::var("URL_SHORTEN_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let addr = listen_addr.parse::<SocketAddr>()?;

    let service = create_service().await?;

    println!("URL Shortening Service listening on {}", addr);

    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
