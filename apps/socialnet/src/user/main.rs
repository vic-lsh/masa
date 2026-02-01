use tracing::info;
use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use mongodb::Client as MongoClient;
use deadpool_redis::{Config, Runtime};

mod server;
use server::{social_network::user_service_server::UserServiceServer, UserServer};

#[derive(StructOpt, Debug, Clone)]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "USER_SERVICE_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    #[structopt(long, env = "MONGO_URL")]
    pub mongo_url: String,

    #[structopt(long, env = "REDIS_URL")]
    pub redis_url: String,

    #[structopt(long, env = "JWT_SECRET")]
    pub jwt_secret: String,

    #[structopt(long, env = "MACHINE_ID", default_value = "01")]
    pub machine_id: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = Args::from_args();
    launch_masa_server!(UserServiceServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(UserServer, std::net::SocketAddr), Box<dyn std::error::Error>> {
    let mongo_client = MongoClient::with_uri_str(&args.mongo_url).await?;
    info!("Successfully connected to MongoDB.");

    let cfg = Config::from_url(args.redis_url);
    let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
    info!("Successfully created Redis connection pool.");

    // Test the pool
    {
        let mut conn = pool.get().await.expect("Failed to get Redis connection from pool");
        let _: () = deadpool_redis::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .expect("Redis PING failed");
        info!("Successfully tested Redis connection pool.");
    }

    let user_service = UserServer::new(
        mongo_client.database("user").collection("user"),
        pool,
        args.jwt_secret,
        args.machine_id,
    );

    let addr = args.listen_addr.parse()?;
    info!("User Service listening on {}", addr);

    Ok((user_service, addr))
}