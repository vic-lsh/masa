// Use the 'tracing' macros (info!, error!, etc.)
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod server;
use server::{social_network::user_service_server::UserServiceServer, UserServer};

use mongodb::Client as MongoClient;
use std::env;
use tonic::transport::Server;

use deadpool_redis::{Config, Runtime};

// Import the AsyncCommands trait from deadpool's re-exported redis crate
// use deadpool_redis::redis::AsyncCommands;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize centralized logging
    // This will collect logs from your app, tonic, mongodb, and redis.
    // You can control log level by setting RUST_LOG=info
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // 2. Read ALL necessary environment variables
    let listen_addr =
        env::var("USER_SERVICE_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let mongo_url = env::var("MONGO_URL").expect("MONGO_URL environment variable must be set");
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL environment variable must be set");
    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET environment variable must be set");
    let machine_id = env::var("MACHINE_ID").unwrap_or_else(|_| "01".to_string());

    // 3. Initialize MongoDB client
    let mongo_client = MongoClient::with_uri_str(&mongo_url).await?;
    info!("Successfully connected to MongoDB.");

    // --- THIS IS THE ROBUST REDIS CONNECTION POOL ---
    // 4. Initialize Redis connection pool
    let cfg = Config::from_url(redis_url);
    let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
    info!("Successfully created Redis connection pool.");

    // Test the pool by getting a connection
    {
        let mut conn = pool
            .get()
            .await
            .expect("Failed to get Redis connection from pool");
        // --- THIS IS THE FIX ---
        // Use the .ping() method from the AsyncCommands trait
        let _: () = deadpool_redis::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .expect("Redis PING failed");
        info!("Successfully tested Redis connection pool.");
    }

    // 5. Create your service instance
    let user_service = UserServer::new(
        mongo_client.database("user").collection("user"),
        pool, // Pass the pool, not a single connection
        jwt_secret,
        machine_id,
    );

    // 6. Start the gRPC server
    let addr = listen_addr.parse()?;
    info!("User Service listening on {}", addr);

    Server::builder()
        .add_service(UserServiceServer::new(user_service))
        .serve(addr)
        .await?;

    Ok(())
}
