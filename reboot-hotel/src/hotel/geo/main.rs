pub mod server;

use std::sync::Arc;

use hyper::rt::Exec;
use tonic::{masa::AsyncTaskMetadata, transport::Server};

use reboot_hotel::{init_logging, ExecImpl};

use server::hotel::geo::geo_server::GeoServer;
use server::GeoImpl;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let geo_addr = "[::1]:8662".parse().expect("Failed to parse address");

    static SMOL_EX: smol::Executor<'static, AsyncTaskMetadata> = smol::Executor::new();
    let ex = Arc::new(ExecImpl::new(&SMOL_EX));
    let ex_clone = ex.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(ex_clone.run());
    });

    let geo = GeoImpl::new();
    log::info!("Server listening on {}...", geo_addr);
    Server::builder()
        .add_service(GeoServer::new(geo))
        .serve_with_executor(geo_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
