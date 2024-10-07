use server::masa::geo::geo_server::GeoServer;
use server::GeoImpl;
use tonic::transport::Server;

pub mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let geo_addr = "[::]:8662".parse().expect("Failed to parse address");

    let geo = GeoImpl::new();
    eprintln!("Server listening on {}...", geo_addr);
    Server::builder()
        .add_service(GeoServer::new(geo))
        .serve(geo_addr)
        .await?;

    Ok(())
}
