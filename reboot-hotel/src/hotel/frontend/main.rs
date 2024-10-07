use server::masa::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;
use tonic::transport::Server;

pub mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frontend_addr = "[::]:8660".parse().expect("Failed to parse address");
    let search_addr = "http://node2:8661".to_string();
    let profile_addr = "http://node2:8664".to_string();

    let frontend = FrontendImpl::new(search_addr, profile_addr).await;
    eprintln!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve(frontend_addr)
        .await?;

    Ok(())
}
