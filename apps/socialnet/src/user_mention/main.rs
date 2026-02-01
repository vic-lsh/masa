use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use socialnet::user_mention::server::user_mention_service::user_mention_service_server::UserMentionServiceServer;
use socialnet::user_mention::server::UserMentionServiceImpl;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "USER_MENTION_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    #[structopt(long, env = "MONGO_URL")]
    pub mongo_url: String,
    
    #[structopt(long, env = "MEMCACHED_URL")]
    pub memcached_url: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(UserMentionServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(UserMentionServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    let service = UserMentionServiceImpl::new(&args.mongo_url, &args.memcached_url).await?;
    log::info!("User Mention Service listening on {}", addr);
    Ok((service, addr))
}