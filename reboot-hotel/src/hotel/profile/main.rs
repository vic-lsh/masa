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

use config::Config;
use reboot_hotel::{init_logging, ExecImpl};

use server::hotel::profile::profile_server::ProfileServer;
use server::ProfileImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let _args = Args::from_args();
    // let file = File::open(args.config).expect("Failed to open file");
    // let reader = BufReader::new(file);
    // let cfg: Config = serde_json::from_reader(reader)?;
    // log::info!("Hotel config: {:?}", cfg);

    // let profile_addr = format!("[::1]:{}", cfg.profile_port)
    //     .parse()
    //     .expect("Failed to parse address");

    // let profile = ProfileImpl::new(
    //     cfg.hotels,
    //     cfg.payload,
    //     cfg.profile_memcached_addr,
    //     cfg.cache_conn,
    //     cfg.cache_miss_rate,
    //     cfg.profile_mongodb_addr,
    // )
    // .await?;

    let profile_addr = "[::1]:8664".parse().expect("Failed to parse address");

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

    let profile = ProfileImpl::new().await?;
    log::info!("Server listening on {}...", profile_addr);
    Server::builder()
        .add_service(ProfileServer::new(profile))
        .serve_with_executor(profile_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
