#![allow(dead_code)]

use rand::Rng;
use unique_id_service::unique_id_service_client::UniqueIdServiceClient;
use unique_id_service::UniqueIdRequest;

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let mut client = UniqueIdServiceClient::connect("http://[::1]:50051").await?;

    // let request: tonic::Request<UniqueIdRequest> = generate_unique_id_request();

    // let response = client.compose_unique_id(request).await?;

    // println!("RESPONSE={:?}", response);

    // Ok(())
    let server_addr = env::var("UNIQUE_ID_SERVICE_ADDR")
        .unwrap_or_else(|_| "http://[::1]:50051".to_string());

    println!("Connecting to Unique ID Service at {}...", server_addr);
    let mut client = UniqueIdServiceClient::connect(server_addr).await?;

    let request = generate_unique_id_request();
    let response = client.compose_unique_id(request).await?;

    println!("RESPONSE={:?}", response);

    Ok(())
}

fn generate_unique_id_request() -> tonic::Request<UniqueIdRequest> {
    // Initialize the random number generator
    let mut rng = rand::thread_rng();

    // Generate a random ID between 1 and 100 (inclusive)
    let random_id = rng.gen_range(1..=100);

    // Create the UniqueIdRequest with the generated ID
    let unique_id_request = UniqueIdRequest { id: random_id };

    // Wrap it in a tonic::Request
    tonic::Request::new(unique_id_request)
}
