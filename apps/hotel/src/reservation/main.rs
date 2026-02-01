#[path = "../config.rs"]
pub mod config;
mod db;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use app_utils::{launch_masa_server, config::PolicyArgs};
use config::HotelConfig;
use server::hotel_tonic::reservation::reservation_server::ReservationServer;
use server::ReservationImpl;
use tonic::masa::TonicPolicy;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    launch_masa_server!(args.policy, run_server, args)
}

fn run_server<P: TonicPolicy>(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Build the runtime with the specific policy
    let rt = tokio::runtime::Builder::new_current_thread()
        .policy::<P>()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let cfg: HotelConfig = {
            let file = File::open(args.config).expect("Failed to open file");
            let reader = BufReader::new(file);
            serde_json::from_reader(reader)?
        };

        let HotelConfig { reservation, .. } = cfg;

        let reservation_addr = format!("{}:{}", "[::]", reservation.port)
            .parse()
            .expect("Failed to parse address");
        log::warn!("Server listening on {}...", reservation_addr);
        let reservation_service = ReservationImpl::new(reservation).await?;
        
        // Instantiate the service with the specific Hooks from the Policy
        let service = ReservationServer::<_, <P as TonicPolicy>::Hooks>::with_custom_context(reservation_service);

        Server::builder_with_policy::<P>()
            .add_service(service)
            .serve_with_masa(reservation_addr)
            .await?;

        Ok(())
    })
}