mod server;
use server::{social_network::user_service_server::UserServiceServer, UserServer};

use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Server;
use mongodb::Client as MongoClient;
use redis_async::client as redis_client;
use url::Url; // <-- Add this use statement

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Read ALL necessary environment variables
    let listen_addr = env::var("USER_SERVICE_LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let mongo_url = env::var("MONGO_URL")
        .expect("MONGO_URL environment variable must be set");
    let redis_url_str = env::var("REDIS_URL")
        .expect("REDIS_URL environment variable must be set");
    let jwt_secret = env::var("JWT_SECRET")
        .expect("JWT_SECRET environment variable must be set");
    let machine_id = env::var("MACHINE_ID")
        .unwrap_or_else(|_| "01".to_string());

    // 2. Initialize database clients
    let mongo_client = MongoClient::with_uri_str(&mongo_url).await?;
    println!("Successfully connected to MongoDB.");
    
    // --- THIS IS THE CORRECTED REDIS CONNECTION ---
    // Parse the full URL string from the environment variable
    let redis_url = Url::parse(&redis_url_str)?;
    let redis_host = redis_url.host_str().expect("Redis URL must have a host");
    let redis_port = redis_url.port().expect("Redis URL must have a port");
    
    // Pass the parsed host and port to the function
    let redis_conn = redis_client::paired_connect(redis_host, redis_port).await?;
    println!("Successfully connected to Redis.");

    // 3. Create your service instance
    // let user_service = UserServer {
    //     mongo_user_collection: mongo_client.database("user").collection("user"),
    //     redis_conn: Arc::new(Mutex::new(redis_conn)),
    //     jwt_secret,
    //     machine_id,
    // };
    let user_service = UserServer::new(
        mongo_client.database("user").collection("user"),
        Arc::new(Mutex::new(redis_conn)),
        jwt_secret,
        machine_id,
    );


    // 4. Start the gRPC server
    let addr = listen_addr.parse()?;
    println!("User Service listening on {}", addr);

    Server::builder()
        .add_service(UserServiceServer::new(user_service))
        .serve(addr)
        .await?;

    Ok(())
}
