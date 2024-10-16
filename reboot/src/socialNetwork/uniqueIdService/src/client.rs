#![allow(dead_code)]

use rand::Rng;
use unique_id_service::greeter_client::GreeterClient;
use unique_id_service::UniqueIdRequest;

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = GreeterClient::connect("http://[::1]:50051").await?;

    let request: tonic::Request<UniqueIdRequest> = generate_unique_id_request();

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
