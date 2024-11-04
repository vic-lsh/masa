pub mod server;

use std::path::PathBuf;
use std::sync::Arc;

use hyper::rt::Exec;
use structopt::StructOpt;
use tonic::{masa::AsyncTaskMetadata, transport::Server};

use reboot_hotel::{init_logging, ExecImpl};
use server::hotel::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let _args = Args::from_args();

    let frontend_addr = "[::1]:8660".parse().expect("Failed to parse address");
    let search_addr = "http://[::1]:8661".to_string();
    let profile_addr = "http://[::1]:8664".to_string();

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

    let frontend = FrontendImpl::new(search_addr, profile_addr).await;
    log::info!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve_with_executor(frontend_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
