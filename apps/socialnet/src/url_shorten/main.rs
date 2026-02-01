use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use socialnet::url_shorten::url_shorten_service_server::UrlShortenServiceServer;
use socialnet::url_shorten::server::UrlShortenServiceImpl;
use socialnet::url_shorten::db::initialize_database;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "URL_SHORTEN_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    #[structopt(long, env = "MONGO_URL")]
    pub mongo_url: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(UrlShortenServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(UrlShortenServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    
    let mongo_client = initialize_database(&args.mongo_url).await?;
    let service = UrlShortenServiceImpl::new(mongo_client);
    
    log::info!("URL Shortening Service listening on {}", addr);
    Ok((service, addr))
}