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
use server::hotel::search::search_server::SearchServer;
use server::SearchImpl;

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

    let search_addr = "[::1]:8661".parse().expect("Failed to parse address");
    let geo_addr = "http://[::1]:8662".to_string();
    let rate_addr = "http://[::1]:8663".to_string();

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

    let search = SearchImpl::new(geo_addr, rate_addr).await;
    log::warn!("Server listening on {}...", search_addr);
    Server::builder()
        .add_service(SearchServer::new(search))
        .serve_with_executor(search_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
