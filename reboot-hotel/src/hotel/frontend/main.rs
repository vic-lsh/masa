#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::rt::Exec;
use structopt::StructOpt;
use tonic::{masa::AsyncTaskMetadata, transport::Server};

use config::HotelConfig;
use reboot_hotel::{init_logging, ExecImpl};
use server::hotel::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::warn!("Hotel config: {:?}", cfg);

    let frontend_addr = "[::1]:8660".parse().expect("Failed to parse address");
    let search_addr = "http://[::1]:8661".to_string();
    let reservation_addr = "http://[::1]:8665".to_string();
    let profile_addr = "http://[::1]:8664".to_string();
    let user_addr = "http://[::1]:8666".to_string();

    static SMOL_EX: smol::Executor<'static, AsyncTaskMetadata> = smol::Executor::new();
    let ex = Arc::new(ExecImpl::new(&SMOL_EX));
    for _ in 0..cfg.executor_threads {
        let ex_clone = ex.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(ex_clone.run());
        });
    }

    let frontend = FrontendImpl::new(search_addr, reservation_addr, profile_addr, user_addr).await;
    log::warn!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve_with_executor(frontend_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
