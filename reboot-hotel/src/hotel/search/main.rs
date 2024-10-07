use server::masa::search::search_server::SearchServer;
use server::SearchImpl;
use tonic::transport::Server;

pub mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let search_addr = "[::]:8661".parse().expect("Failed to parse address");
    let geo_addr = "http://[::1]:8662".to_string();
    let rate_addr = "http://[::1]:8663".to_string();

    let search = SearchImpl::new(geo_addr, rate_addr).await;
    eprintln!("Server listening on {}...", search_addr);
    Server::builder()
        .add_service(SearchServer::new(search))
        .serve(search_addr)
        .await?;

    Ok(())
}
